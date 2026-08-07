//! Which projects this installation syncs, where they are, and the credentials
//! that make that true.
//!
//! [`wobu_sync::ticket`] says at length where a ticket is kept and that it is
//! not there: `wobu-sync` is a transport crate with no filesystem code in it,
//! and `identity.rs` argues that the moment a *file* can supply a secret to that
//! crate, the next question is which file and somebody's answer is one inside
//! the project. So the shell persists it, which is this module, and the shell is
//! the right owner because it already keeps provider keys and already owns
//! app-data paths.
//!
//! ## Never in the project folder
//!
//! `app_data_dir()/sync/shares.json`, beside the index and the recents list and
//! nowhere near a world. A project folder is copied to a NAS, a USB stick and a
//! git repo, so a grant inside one is a grant handed to everybody who can mount
//! it — and since there is no revocation, handed to them permanently. The file
//! is written `0600` on Unix for the same reason `keys.rs` puts provider keys in
//! the credential store rather than in a config file: the threat is not a remote
//! attacker, it is the other accounts on a shared machine and the backup tool
//! that syncs a home directory somewhere.
//!
//! It is a *file* rather than a keychain entry, and that is a deliberate step
//! down from where the ed25519 secret lives. The identity key is the thing that
//! *is* this peer and its disclosure is unrecoverable; a grant is a bearer
//! capability checked by `wobu/sync/1` before a project is honoured. Putting a
//! list that changes whenever a project is shared into a store that prompts on
//! every read, on Linux, would be a dialog per sync poll. This paragraph is the
//! record of that persistence trade.
//!
//! ## Why the whole ticket is kept and not just an address
//!
//! An accepted ticket is how this machine knows *who to dial* for a project
//! nobody has open. Keeping the canonical string rather than the parsed address
//! means the entry in this file and the string in somebody's chat log are the
//! same characters, which is what lets a support conversation say "paste what is
//! in the file" — and it keeps the grant every outbound ticket connection must
//! present.
//!
//! ## What this is not
//!
//! Not the recents list. `wobu_store::recent` holds what a person last opened,
//! is truncated to a dozen entries and is explicitly a hint that may be stale.
//! A share is a standing commitment to keep a world in step with somebody, it
//! outlives being opened, and losing one silently is losing a collaborator. Two
//! lists, two lifetimes, and folding them together would mean the twelfth
//! project a user opened stopping syncing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_store::paths;
use wobu_sync::{Grant, Ticket};

/// One project this installation is willing to sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    pub project: Id,
    /// Where the folder is on *this* machine. Machine-specific, like a recents
    /// entry, and stale exactly as often — a share that has been unmounted or
    /// moved is an ordinary state and not an error.
    pub root: PathBuf,
    /// The grant every ticket this machine mints for this project carries.
    ///
    /// Minted once and kept, because a project shared a second time must be
    /// shared with the *same* grant or the ticket a collaborator pasted into
    /// their notes last month stops being one this machine would honour. That
    /// the accept path checks it is precisely why it must not drift.
    pub grant: Grant,
    /// Tickets accepted for this project: who to dial.
    ///
    /// A list rather than one, because a world with three collaborators is three
    /// tickets and M3 is pure peer-to-peer with no seed node — there is nobody
    /// to learn the third peer from except the person who sent the ticket.
    #[serde(default)]
    pub peers: Vec<Ticket>,
}

/// Every share, and the file they came from.
#[derive(Debug)]
pub struct Shares {
    path: PathBuf,
    entries: Vec<Share>,
}

impl Shares {
    /// Read the list, or start an empty one.
    ///
    /// A missing file is the ordinary first-run state. A file that will not
    /// parse is treated the same way, and that is a real trade: a hand-edited or
    /// half-written `shares.json` costs the user their share list rather than
    /// blocking the app from starting. It is the same call `recent::list` makes,
    /// and the alternative — refusing to start sync because one line is wrong —
    /// is worse for the one user who hits it and worse for everyone debugging it.
    pub fn load() -> Shares {
        Shares::load_from(default_path())
    }

    pub fn load_from(path: PathBuf) -> Shares {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Shares { path, entries }
    }

    pub fn all(&self) -> &[Share] {
        &self.entries
    }

    pub fn get(&self, project: Id) -> Option<&Share> {
        self.entries.iter().find(|s| s.project == project)
    }

    /// Start sharing a project, or update where it lives.
    ///
    /// Keyed by project ULID rather than by path, because one ULID is one world
    /// however many folders it has been in — a project that moved should update
    /// its entry, not gain a second one that dials the same peers about the same
    /// world from a directory that no longer exists.
    ///
    /// The grant is preserved when an entry already has one. Regenerating it on
    /// every call would invalidate every ticket already pasted into a chat
    /// window, silently, and the user's only symptom would be a collaborator who
    /// stopped syncing.
    pub fn share(&mut self, project: Id, root: &Path) -> &Share {
        let index = match self.entries.iter().position(|s| s.project == project) {
            Some(index) => {
                self.entries[index].root = root.to_path_buf();
                index
            }
            None => {
                self.entries.push(Share {
                    project,
                    root: root.to_path_buf(),
                    grant: Grant::generate(),
                    peers: Vec::new(),
                });
                self.entries.len() - 1
            }
        };
        &self.entries[index]
    }

    /// Record a ticket somebody sent us for a project.
    ///
    /// Deduplicated by the peer's endpoint id rather than by the whole ticket:
    /// a collaborator who re-shared after their address changed sends a
    /// *different* string for the *same* peer, and keeping both would leave this
    /// machine dialling an address nobody answers on for ever. The newer one
    /// wins because it is the one they just sent.
    pub fn invite(&mut self, project: Id, root: &Path, ticket: Ticket) {
        self.share(project, root);
        let Some(entry) = self.entries.iter_mut().find(|s| s.project == project) else { return };
        entry.peers.retain(|t| t.peer() != ticket.peer());
        entry.peers.push(ticket);
    }

    /// Stop syncing a project. Returns whether there was one.
    pub fn forget(&mut self, project: Id) -> bool {
        let before = self.entries.len();
        self.entries.retain(|s| s.project != project);
        before != self.entries.len()
    }

    /// Write the list back.
    ///
    /// Best-effort and reported rather than propagated: a share that could not
    /// be written down still works for this run, and failing the command the
    /// user just ran would be worse than losing the entry on restart. The caller
    /// logs it.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)
            .expect("a share is ids, paths and canonical ticket strings");
        std::fs::write(&self.path, json)?;
        paths::restrict(&self.path)
    }
}

fn default_path() -> PathBuf {
    paths::app_data_dir().join("sync").join("shares.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::tests::scratch;

    fn ticket(project: Id) -> Ticket {
        use iroh::{EndpointAddr, SecretKey};
        let addr = EndpointAddr::from_parts(SecretKey::generate().public(), []);
        Ticket::new(project, addr, Grant::generate())
    }

    #[test]
    fn a_share_survives_a_restart_with_its_grant_intact() {
        // The whole point of the file. A grant that did not come back would mean
        // every ticket already pasted into a chat window naming a credential this
        // machine no longer has — and since there is no revocation and no error
        // path, the symptom would be a collaborator who silently stopped syncing.
        let dir = scratch("shares-restart");
        let path = dir.join("shares.json");

        let project = wobu_core::new_id();
        let mut shares = Shares::load_from(path.clone());
        let grant = shares.share(project, Path::new("/worlds/Ashfall.wobu")).grant;
        shares.invite(project, Path::new("/worlds/Ashfall.wobu"), ticket(project));
        shares.save().unwrap();

        let reloaded = Shares::load_from(path);
        let entry = reloaded.get(project).expect("the share came back");
        assert_eq!(entry.grant, grant);
        assert_eq!(entry.peers.len(), 1);
        assert_eq!(entry.root, Path::new("/worlds/Ashfall.wobu"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sharing_a_project_twice_keeps_the_grant_it_already_had() {
        // Re-sharing is the ordinary way a user hands the string to a second
        // collaborator. Minting a fresh grant there would invalidate the first
        // one's ticket with no error, no event and nothing on screen.
        let dir = scratch("shares-regrant");
        let mut shares = Shares::load_from(dir.join("shares.json"));
        let project = wobu_core::new_id();

        let first = shares.share(project, Path::new("/a")).grant;
        let second = shares.share(project, Path::new("/b")).grant;

        assert_eq!(first, second);
        // …and the path is allowed to move, because one ULID is one world
        // however many folders it has been in.
        assert_eq!(shares.get(project).unwrap().root, Path::new("/b"));
        assert_eq!(shares.all().len(), 1, "a moved project gained a second entry");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_peer_that_reshared_from_a_new_address_replaces_its_old_ticket() {
        // A collaborator whose relay or LAN address changed sends a different
        // string for the same endpoint id. Keeping both would leave this machine
        // dialling somewhere nobody answers on every poll, for ever.
        let dir = scratch("shares-reshare");
        let mut shares = Shares::load_from(dir.join("shares.json"));
        let project = wobu_core::new_id();
        let root = Path::new("/worlds/Ashfall.wobu");

        let peer = iroh::SecretKey::generate();
        let old = Ticket::new(
            project,
            iroh::EndpointAddr::from_parts(peer.public(), []),
            Grant::generate(),
        );
        let new = Ticket::new(
            project,
            iroh::EndpointAddr::from_parts(peer.public(), [])
                .with_relay_url("https://relay.example/".parse().unwrap()),
            Grant::generate(),
        );

        shares.invite(project, root, old);
        shares.invite(project, root, new.clone());

        let peers = &shares.get(project).unwrap().peers;
        assert_eq!(peers.len(), 1, "one peer, two entries");
        assert_eq!(peers[0], new);

        // A genuinely different peer is a second entry, not a replacement.
        shares.invite(project, root, ticket(project));
        assert_eq!(shares.get(project).unwrap().peers.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shares_file_that_will_not_parse_costs_the_list_and_not_the_app() {
        // The trade, stated as a test so it is a decision rather than an
        // accident. Refusing to start sync because one line is wrong is worse
        // for the user who hits it than losing a list they can rebuild.
        let dir = scratch("shares-garbage");
        let path = dir.join("shares.json");
        std::fs::write(&path, "{ not json at all").unwrap();

        assert!(Shares::load_from(path).all().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch("shares-mode");
        let path = dir.join("shares.json");
        let mut shares = Shares::load_from(path.clone());
        shares.share(wobu_core::new_id(), Path::new("/a"));
        shares.save().unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "group or other can read {path:?}: {mode:o}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
