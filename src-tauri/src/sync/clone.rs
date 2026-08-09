//! Turning an accepted ticket into a project folder on this machine.
//!
//! A clone is scaffolded before a single body arrives, so a transfer that fails
//! halfway leaves a folder this machine can recognise and remove rather than a
//! half-project it would later try to open.

use super::*;

/// The body of [`sync_accept`], taking a plain reference.
///
/// Split out so its future can be *measured*, which
/// [`the_accept_future_fits_a_windows_stack`](tests::the_accept_future_fits_a_windows_stack)
/// does: `tauri::State` has no public constructor, so a test cannot build the
/// command's own future, and the size of this one is the thing #149 turned on.
pub(super) async fn accept_ticket(
    sync: &SyncState,
    token: Option<String>,
    destination: Option<String>,
    cancel: Option<bool>,
) -> CommandResult<Option<Accepted>> {
    if cancel.unwrap_or(false) {
        sync.cancel_accept();
        return Ok(None);
    }
    // Three breadcrumbs bracket this command, and they are here for #149: on
    // Windows the process dies during accept leaving a log that stops at
    // startup, and every branch below used to be able to return without
    // recording anything. Which of these lines is *last* in a crash log says
    // where the process got to — the probe is the only step between the click
    // and the folder picker, so "read" but no "asking where to put it" means it
    // died decoding the ticket, and the reverse means it died after this
    // command had already returned. Never the token itself: it is a credential.
    diag::info("sync: a ticket was pasted; reading it");
    let manager = sync.manager()?;
    let token = token.ok_or_else(|| WobuError::new(Code::Invalid, "Paste a Wobu share link."))?;
    let ticket: Ticket = token.parse().map_err(WobuError::from)?;
    let alias = ticket.alias();
    let project = ticket.project();

    if manager.accept(&ticket) == Disposition::Join {
        diag::info(format!("sync: joined {project} with {alias}"));
        let root = manager.root_of(project).map(|path| path.to_string_lossy().into_owned());
        return Ok(Some(Accepted { project, alias, joined: true, root }));
    }

    let Some(destination) = destination else {
        diag::info(format!(
            "sync: {project} from {alias} is not on this machine; asking where to put it"
        ));
        return Ok(Some(Accepted { project, alias, joined: false, root: None }));
    };
    diag::info(format!("sync: cloning {project} from {alias} into {destination}"));
    // Boxed, and this is #149's fix rather than a tidy-up.
    //
    // An `async fn`'s future is sized for its largest branch, whichever branch
    // actually runs. Inlined, `clone_into`'s dominates — it holds the whole
    // transfer, and `run_ticket`'s state machine alone measures ~130 KB — so
    // *every* call to this command built that much on the stack, including the
    // destination-less probe that returns without touching any of it.
    //
    // The stack it was built on is the main thread's: a command is invoked from
    // a WebView2 callback, and Tauri constructs the future there before handing
    // it to the runtime. Windows reserves 1 MiB for a main thread against
    // Linux's 8 MiB, which is exactly why this only ever died on Windows, and
    // why it died with `STATUS_STACK_OVERFLOW` — a fault no panic hook can see,
    // leaving a log that just stops.
    //
    // `Box::pin` puts the transfer on the heap, and it is allocated when this
    // future is first *polled* — by then execution is on a runtime worker with
    // a stack of its own, so the main thread never carries it at all.
    Box::pin(clone_into(sync, &manager, &ticket, destination, project, alias)).await
}

/// Create the folder, pull the world into it, and undo both if that fails.
///
/// Deliberately its own function so the caller can box it; see the comment at
/// the call site. Kept whole rather than split further because the scaffold and
/// the cleanup are two halves of one guarantee: a clone that fails leaves
/// nothing behind that a retry would trip over.
pub(super) async fn clone_into(
    sync: &SyncState,
    manager: &Arc<SyncManager>,
    ticket: &Ticket,
    destination: String,
    project: Id,
    alias: String,
) -> CommandResult<Option<Accepted>> {
    // The lease is RAII: every return path, including scaffold validation,
    // releases the operation slot. A Cancel during this synchronous step is a
    // stored Notify permit when the network wait starts.
    let accept = sync.begin_accept()?;
    let scaffold = create_clone_scaffold(Path::new(&destination), project)?;
    let root = scaffold.root.clone();
    manager.accept_clone(ticket, &root);

    let downloaded = tokio::select! {
        result = manager.run_ticket(project, ticket) => result,
        () = accept.cancelled() => Err(WobuError::new(Code::Cancelled, "Accepting the shared project was cancelled.")),
    };
    match downloaded {
        Ok(()) => {
            scaffold.complete();
            manager.start_poller(project);
            Ok(Some(Accepted {
                project,
                alias,
                joined: false,
                root: Some(root.to_string_lossy().into_owned()),
            }))
        }
        // A peer going offline during the first round does not make the local
        // project unusable. The scaffold is a valid project, every node write
        // is atomic, and every blob is verified and renamed individually. Open
        // what arrived and let the retained ticket's poller carry on later.
        // Returning an error here used to strand the newly created folder on
        // the launcher and force the user to find and open it by hand.
        Err(error) if error.retryable => {
            diag::error(format!(
                "sync: initial clone round did not finish; opening the partial replica: {}",
                error.message
            ));
            scaffold.complete();
            manager.start_poller(project);
            Ok(Some(Accepted {
                project,
                alias,
                joined: false,
                root: Some(root.to_string_lossy().into_owned()),
            }))
        }
        Err(error) => {
            cleanup_clone(manager, project, &root);
            Err(error)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloneMarker {
    project: Id,
    nonce: Id,
}

pub(super) struct CloneScaffold {
    pub(super) root: PathBuf,
    pub(super) marker: PathBuf,
}

impl CloneScaffold {
    fn complete(self) {
        if let Err(error) = std::fs::remove_file(&self.marker) {
            diag::error(format!(
                "sync: could not remove completed clone marker {}: {error}",
                self.marker.display()
            ));
        }
    }
}

pub(super) fn create_clone_scaffold(parent: &Path, project: Id) -> CommandResult<CloneScaffold> {
    if !parent.is_dir() {
        return Err(WobuError::new(
            Code::Invalid,
            "Choose an existing destination folder for the shared project.",
        ));
    }
    let short = project.to_string().chars().take(8).collect::<String>().to_lowercase();
    let root = parent.join(format!("shared-{short}.wobu"));
    let created_root = match std::fs::create_dir(&root) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(wobu_store::Error::io(&root, error).into()),
    };
    let metadata =
        std::fs::symlink_metadata(&root).map_err(|error| wobu_store::Error::io(&root, error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(WobuError::new(
            Code::Invalid,
            "The clone destination is not a regular folder.",
        ));
    }
    let marker = root.join(".wobu/accepting.json");
    let created = (|| -> wobu_store::Result<()> {
        let path = root.join("project.json");
        if !created_root {
            // Validate ownership before creating even one child. An unrelated
            // collision, including one with hostile child symlinks, remains
            // byte-for-byte untouched when refused.
            let marker_bytes =
                std::fs::read(&marker).map_err(|error| wobu_store::Error::io(&marker, error))?;
            let clone_marker: CloneMarker = serde_json::from_slice(&marker_bytes)?;
            if clone_marker.project != project {
                return Err(wobu_store::Error::AlreadyExists(root.clone()));
            }
            let existing: ProjectMeta = serde_json::from_slice(
                &std::fs::read(&path).map_err(|error| wobu_store::Error::io(&path, error))?,
            )?;
            if existing.id != project {
                return Err(wobu_store::Error::AlreadyExists(root.clone()));
            }
        }
        for rel in [
            "nodes",
            "assets/originals",
            "assets/thumbs",
            "generations",
            ".wobu/tmp",
            ".wobu/sessions",
        ] {
            wobu_store::paths::ensure_dir(&root.join(rel))?;
        }
        let meta = ProjectMeta {
            id: project,
            name: format!("Shared project {short}"),
            schema_version: SCHEMA_VERSION,
            created_at: chrono::Utc::now(),
            providers: serde_json::Map::new(),
            // Match the store's default for a newly created project. The
            // canonical metadata is not part of the node-sync protocol yet.
            spend_ceiling_usd_micros: Some(10_000_000),
        };
        if created_root {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| wobu_store::Error::io(&path, error))?;
            file.write_all(&serde_json::to_vec_pretty(&meta)?)
                .map_err(|error| wobu_store::Error::io(&path, error))?;
            file.sync_all().map_err(|error| wobu_store::Error::io(&path, error))?;
            let clone_marker = CloneMarker { project, nonce: wobu_core::new_id() };
            std::fs::write(&marker, serde_json::to_vec_pretty(&clone_marker)?)
                .map_err(|error| wobu_store::Error::io(&marker, error))?;
        }
        Ok(())
    })();
    if let Err(error) = created {
        // Never recursively delete here. A person, another process, or a
        // completed atomic sync write may already have placed recoverable data
        // in this path. Only a verified marker may resume it.
        return Err(error.into());
    }
    Ok(CloneScaffold { root, marker })
}

fn cleanup_clone(manager: &SyncManager, project: Id, root: &Path) {
    if let Err(error) = manager.unshare(project) {
        diag::error(format!(
            "sync: could not discard cancelled clone registration: {}",
            error.message
        ));
    }
    // Keep the marker and every downloaded file. Cancellation can land after
    // an atomic node write; recursively deleting the directory would turn
    // Cancel into data loss. Selecting the same parent later resumes only after
    // marker and project-id validation.
    diag::info(format!("sync: kept resumable partial clone at {}", root.display()));
}
