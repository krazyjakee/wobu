//! Enhance: a node's notes, and the stack above it, turned into a description
//! somebody is then asked to approve.
//!
//! ## Three things kept apart on purpose
//!
//! **The call runs in the job queue, and nothing else does.** `enhance_start`
//! resolves the stack, builds the context, picks the provider, and hands a
//! [`Task`] to `wobu-jobs` — then returns a job id, before the work starts. Every
//! property that matters afterwards is the queue's: cancellation that aborts the
//! request rather than discarding its answer, a free retry for a rate limit, and
//! a *held* retry for a response that was paid for and came back broken. None of
//! that is re-implemented here and none of it could be, correctly, twice.
//!
//! **Nothing this module streams reaches disk.** The provider emits fragments of
//! the JSON document; they are read incrementally
//! ([`wobu_llm::read_partial`]) and emitted as [`ENHANCE_DELTA`] for the editor
//! to type out, and that is all they are ever used for. The only route to a
//! description a node will hold is `EnhanceOutcome::result`, which is `Ok` only
//! for a response that arrived whole and passed its kind's schema.
//!
//! **A finished call does not write anything either.** It leaves the validated
//! description in [`Pending`] and says so on `job:done`; the user reads it, edits
//! it or rejects it, and `enhance_accept` is what writes — through
//! `Project::accept_enhanced`, the only supported way to write a machine
//! description and the thing that stamps the upstream versions it was built from.
//! That split is also why the task holds no reference to [`AppState`]: a job
//! outlives the project it was started for, and `state.rs` says there must be no
//! path from one to the other.
//!
//! See `docs/04-influence-engine.md`, whose "Enhance pipeline" section this is.

mod context;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, State};
use wobu_core::{Description, Id, Node, NodeKind, SectionValue};
use wobu_jobs::{Failure, JobContext, JobId, JobKind, Outcome, Task};
use wobu_llm::{
    AnthropicProvider, Cancel, EnhanceRequest, Error as ProviderError, GeminiProvider,
    QUESTIONS_KEY, TextProvider, Usage, anthropic, gemini, read_partial,
};
use wobu_store::Enhanced;

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::{Keys, Secret};
use crate::state::{AppState, Jobs};

pub use context::SYSTEM;

/// The document so far, on its way to the editor's right-hand pane.
///
/// Not in `wobu_jobs::events`, because it is not a queue event: the queue says
/// what is happening to a *job*, and this says what one particular kind of job
/// has produced. `project:open-progress` is emitted from the shell for the same
/// reason.
pub const ENHANCE_DELTA: &str = "enhance:delta";

/// How often the pane is repainted while a response streams.
///
/// A provider sends fragments far faster than anyone reads, and one bridge
/// message per fragment is a few hundred round trips spent on frames nobody
/// sees. Coalescing to roughly twenty-five a second is still a typing effect and
/// is a fifth of the traffic. `project_open` throttles to whole percentage
/// points for the same reason.
const FRAME: Duration = Duration::from_millis(40);

/// How many finished-but-unanswered descriptions are held.
///
/// Bounded because nothing clears this but an answer: a session that enhances
/// twenty nodes and accepts none would otherwise hold twenty descriptions until
/// the app closed. Eight is comfortably more than anyone reviews at once.
const KEPT: usize = 8;

/* ── what the webview is told ─────────────────────────────────────────────── */

/// A repaint of the pane, sent whole rather than as an append.
///
/// Whole snapshots for the same reason `job:state` sends the whole queue: Tauri
/// events are fire-and-forget, and a pane that accumulated appends would be
/// permanently wrong the first time one was dropped or arrived out of order —
/// wrong in a way that looks like the model having written nonsense.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceDelta {
    pub job_id: JobId,
    pub node_id: Id,
    /// As far as the document has arrived, in the kind's declared section order.
    /// A section that has opened and streamed nothing is present and empty, so
    /// its heading can appear before its text does.
    pub description: Description,
    pub questions: Vec<String>,
}

/// A finished description, waiting for an answer.
///
/// Rides out on `job:done`, and is also what [`enhance_pending`] hands back —
/// one shape rather than two, because they are the same thing seen a moment
/// apart and a pane that reloaded should not have to render a second one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceReady {
    /// Carried even on `job:done`, where `DoneEvent.id` already says it, so that
    /// an entry from the list below is self-describing — the pane matches on
    /// `nodeId` and answers with `jobId`.
    job_id: JobId,
    node_id: Id,
    description: Description,
    /// What the model would otherwise have had to invent, asked instead. Never
    /// written to the node — see `wobu_llm::QUESTIONS_KEY`.
    questions: Vec<String>,
}

/// What `enhance_accept` did.
///
/// [`Accepted::RefusedEdit`] is a result and not an error, and that is the whole
/// point of the type. The description on disk was written by hand and the user
/// has not said to replace it; the right answer is to show them what is about to
/// be overwritten and ask, which is a question with a "yes" in it rather than a
/// failure with an apology.
///
/// A lost save race is *not* here. It comes back as `write.conflict` exactly the
/// way `node_upsert`'s does, because the frontend has one conflict handler and
/// must not need a second.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Accepted {
    Saved { node: Box<Node> },
    RefusedEdit { node: Box<Node> },
}

/* ── descriptions waiting to be answered ──────────────────────────────────── */

/// Validated descriptions that have come back and not yet been accepted or
/// rejected. Managed state, registered in `lib.rs`.
///
/// The `sources` are why this exists rather than the frontend posting everything
/// back. They are the walk the model's context was built from, and the stamp
/// `accept_enhanced` writes has to be made of *that* walk — not of a re-resolve
/// at accept time, which would happily claim the description had seen an
/// upstream edit made in the meantime, and so record a node as fresh against
/// something it was never shown.
#[derive(Clone, Default)]
pub struct Pending(Arc<Mutex<VecDeque<Ready>>>);

#[derive(Clone)]
struct Ready {
    job: JobId,
    /// Which project it was enhanced in. A job outlives the project it was
    /// started for, so without this an accept could be answered against a
    /// different world — one whose nodes it knows nothing about.
    project: Id,
    node_id: Id,
    description: Description,
    /// Held even though nothing on the *write* path reads them, because
    /// [`enhance_pending`] does: a webview that reloaded mid-review has to get
    /// back the questions as well as the description, or half the answer to a
    /// call that has already been paid for is gone.
    questions: Vec<String>,
    sources: Vec<Id>,
}

impl Pending {
    fn remember(&self, ready: Ready) {
        let mut waiting = self.0.lock();
        waiting.retain(|held| held.job != ready.job);
        waiting.push_back(ready);
        while waiting.len() > KEPT {
            waiting.pop_front();
        }
    }

    /// A copy, deliberately: a `RefusedEdit` is answered by calling
    /// `enhance_accept` again with `force`, and taking the description out on the
    /// first call would leave the second with nothing to write.
    fn get(&self, job: JobId) -> Option<Ready> {
        self.0.lock().iter().find(|held| held.job == job).cloned()
    }

    fn forget(&self, job: JobId) {
        self.0.lock().retain(|held| held.job != job);
    }

    /// Everything waiting, newest last, for the project that is open.
    ///
    /// Filtered by project rather than trusting the caller: these outlive the
    /// world they were made in, and a pane that matched one to a node by id
    /// alone would be matching against a world that no longer contains it.
    fn list(&self, project: Id) -> Vec<EnhanceReady> {
        self.0
            .lock()
            .iter()
            .filter(|held| held.project == project)
            .map(|held| EnhanceReady {
                job_id: held.job,
                node_id: held.node_id,
                description: held.description.clone(),
                questions: held.questions.clone(),
            })
            .collect()
    }

    /// Called when a project closes. The bound above makes this a few kilobytes
    /// either way; what it is really for is that a description of someone's
    /// world should not still be in memory after they have shut it.
    pub fn clear(&self) {
        self.0.lock().clear();
    }
}

/* ── commands ─────────────────────────────────────────────────────────────── */

/// Start an Enhance and return its job id, before any of the work happens.
///
/// Everything that can fail *without spending money* fails here, synchronously,
/// so that the failure arrives as an error the user can act on rather than as a
/// job that appears in the status bar and immediately dies:
///
/// - no project, or one that is read-only — a paid call whose answer could never
///   be saved;
/// - a subject that is not in this world;
/// - a provider this build does not have;
/// - **no key on this machine**, which is not a crash and not an internal error.
///   `keys.rs` answers `None` for a collaborator who opened a shared project
///   without their own credentials, and that is an ordinary state:
///   `provider.no_key` is the code the frontend already routes to Settings, and
///   it is not retryable, because pressing "Try again" without pasting a key
///   fails identically.
#[tauri::command]
pub fn enhance_start(
    app: AppHandle,
    state: State<'_, AppState>,
    jobs: State<'_, Jobs>,
    keys: State<'_, Keys>,
    pending: State<'_, Pending>,
    node_id: Id,
) -> CommandResult<String> {
    let (project, kind, label, request, sources, selection) = state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project is read-only, so an enhanced description could not be saved.",
            ));
        }
        let project_id = project.id();
        // Off the already-parsed `project.json`, so nothing here touches the
        // folder — which matters twice over: `state.rs` forbids file IO under
        // this mutex, and a share that is currently unplugged must not stop
        // somebody enhancing from the index they still have.
        let selection = selection(&project.meta().providers);
        let built = context::build(project.world_nodes()?, node_id)
            .ok_or_else(|| no_such_subject(node_id))?;
        Ok((
            project_id,
            built.subject.kind,
            format!("Enhance {}", built.subject.name),
            built.prompt,
            built.sources,
            selection,
        ))
    })?;

    let secret = keys.secret(&selection.provider).ok_or_else(|| no_key(&selection.provider))?;
    let provider = text_provider(&selection.provider, &secret)?;
    let model = selection.model.unwrap_or_else(|| provider.default_model().to_owned());
    diag::info(format!("enhance {node_id} with {} {model}", provider.label()));

    let emitter = app.clone();
    let id = jobs.queue().submit(EnhanceTask {
        node_id,
        kind,
        label,
        project,
        request: EnhanceRequest::new(kind, model, request).with_system(SYSTEM),
        sources,
        provider,
        pending: pending.inner().clone(),
        emit: Arc::new(move |delta| {
            // Dropped deliberately, like every other emit in this app: a window
            // on its way out is not a reason to stop a paid call.
            let _ = emitter.emit(ENHANCE_DELTA, delta);
        }),
    });
    Ok(id.to_string())
}

/// Write a finished description to its node, stamping what it was built from.
///
/// `description` is optional and absent means "what the model sent". The
/// frontend passes one back when the user edited it in the pane before
/// accepting, which is step 3 of the pipeline — and it is the *only* way an
/// edited version gets there, because nothing in this module writes on its own.
///
/// `force` answers a previous [`Accepted::RefusedEdit`], and nothing else ever
/// sets it. It is the user saying they meant to replace a description they wrote
/// by hand.
#[tauri::command]
pub fn enhance_accept(
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
    job_id: String,
    description: Option<Description>,
    force: Option<bool>,
) -> CommandResult<Accepted> {
    let job = job_id_of(&job_id)?;
    let ready = pending.get(job).ok_or_else(|| {
        // `node.invalid` rather than `internal`: this is an argument naming
        // something that is not there, and the ordinary causes are ordinary — a
        // webview that reloaded, or a description that aged out of the eight
        // held. Not retryable, because the same id will keep not being there.
        WobuError::new(
            Code::Invalid,
            "That enhanced description is no longer waiting to be accepted. Run Enhance again.",
        )
        .with_detail(job.to_string())
    })?;

    let outcome = state.with(|project| {
        if project.id() != ready.project {
            return Err(WobuError::new(
                Code::Invalid,
                "That description was enhanced in a different project.",
            ));
        }
        let description = description.unwrap_or(ready.description);
        Ok(project.accept_enhanced(
            ready.node_id,
            description,
            &ready.sources,
            force.unwrap_or(false),
        )?)
    })?;

    match outcome {
        Enhanced::Saved(node) => {
            // Only now. A refusal or a conflict leaves the description waiting,
            // because both of those are questions the user is about to answer
            // and neither has a second copy of the answer anywhere.
            pending.forget(job);
            // No `world:changed` here: the write went into the project folder,
            // so the watcher raises it on its own, and a second one would refetch
            // the whole world twice for one accept.
            Ok(Accepted::Saved { node })
        }
        Enhanced::RefusedEdit(node) => Ok(Accepted::RefusedEdit { node }),
        Enhanced::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/// Throw a finished description away — the "reject" in "stop, edit, or reject".
///
/// Not an error when there is nothing to discard: the user can press it twice,
/// or after a reload, and neither is worth a dialog.
#[tauri::command]
pub fn enhance_discard(pending: State<'_, Pending>, job_id: String) -> CommandResult<()> {
    pending.forget(job_id_of(&job_id)?);
    Ok(())
}

/// Every finished description still waiting for an answer, for the open project.
///
/// The catch-up read, and the same argument `job_list` and `share_offline` make:
/// `job:done` is the live signal, and the one case an event cannot cover is a
/// webview that reloaded after it fired. That case is worse here than anywhere
/// else in the app, because the thing lost was *paid for* — without this, a
/// reload mid-review means running the call again to recover an answer the
/// process is still holding.
///
/// The whole list rather than a lookup by job id, because after a reload the
/// pane has no job id: it knows which node it is showing, and matches on
/// `nodeId`. It is at most [`KEPT`] entries.
///
/// Empty rather than an error when nothing is open — the same reasoning as
/// `presence_peers`. A poll that raced a close is ordinary.
#[tauri::command]
pub fn enhance_pending(
    state: State<'_, AppState>,
    pending: State<'_, Pending>,
) -> Vec<EnhanceReady> {
    match state.peek(|project| project.map(|p| p.id())) {
        Some(project) => pending.list(project),
        None => Vec::new(),
    }
}

fn job_id_of(text: &str) -> CommandResult<JobId> {
    JobId::parse(text).ok_or_else(|| {
        WobuError::new(Code::Internal, "That is not a job id.").with_detail(text.to_owned())
    })
}

fn no_such_subject(id: Id) -> WobuError {
    WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
        .with_detail(id.to_string())
}

/// The failure a collaborator who opened a shared project hits on day one.
///
/// Keys are per installation and `project.json` carries only the selection, so
/// this is expected rather than exceptional — which is why it names the provider
/// and says where the key would go, instead of reporting that something broke.
fn no_key(provider: &str) -> WobuError {
    WobuError::new(
        Code::ProviderNoKey,
        format!(
            "{} is selected for this project, but there is no key for it on this machine. \
             Add one in Settings.",
            label_of(provider),
        ),
    )
}

/* ── which provider ───────────────────────────────────────────────────────── */

/// The capability Enhance asks about, and the key it is stored under in
/// `project.json`'s `providers`.
///
/// Keyed on the capability rather than on the vendor, because the same map has
/// to hold an image backend and a 3D backend as well (#40, #41) and those are
/// *different jobs*, not different vendors — one project can perfectly well use
/// Anthropic for text and ComfyUI for pictures. Enhance asks one question, "who
/// writes text here", and gets one answer; keyed by vendor it would have to
/// enumerate the map and guess which entry meant it.
const TEXT: &str = "text";

/// Which text provider a project has chosen, and which model.
#[derive(Debug, PartialEq)]
struct Selection {
    provider: String,
    /// Absent means the adapter's own default. Model ids move faster than
    /// anything else in `docs/08-providers.md`, so the one this build would pick
    /// is a fact about the adapter rather than about the project.
    model: Option<String>,
}

/// Read the selection out of the open project's `providers`.
///
/// A project that names nothing falls back to the default text provider rather
/// than refusing, because every project created before there was a settings pane
/// for this is in that state and Enhance has to work in it. What must *not*
/// happen is the opposite — a project that named Gemini being quietly given
/// Anthropic — which is why this reads the map at all rather than hardcoding one.
///
/// The map is `Project::meta`'s, which is `project.json` as it was when the
/// project was opened. `reconcile` does not re-read that file, so a selection a
/// collaborator changes on the share is picked up on the next open — which is
/// the right way round: the provider a running session is spending against
/// should not change underneath it between one Enhance and the next.
fn selection(providers: &Map<String, Value>) -> Selection {
    let chosen = providers.get(TEXT).and_then(Value::as_object);
    let field = |name: &str| {
        chosen
            .and_then(|c| c.get(name))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    Selection {
        provider: field("provider").unwrap_or(anthropic::ID).to_owned(),
        model: field("model").map(str::to_owned),
    }
}

/// The adapters this build has, by the id `project.json` and the keychain both
/// use.
///
/// Two matches on the same id, here and in [`label_of`], and they are adjacent
/// so that adding an adapter is visibly two lines rather than one. The split
/// exists because the label is needed *before* a provider can be built: the
/// "no key on this machine" message names the vendor, and there is no key to
/// build one with.
fn text_provider(id: &str, key: &Secret) -> CommandResult<Arc<dyn TextProvider>> {
    let built = match id {
        anthropic::ID => {
            AnthropicProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        gemini::ID => {
            GeminiProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        _ => {
            return Err(WobuError::new(
                Code::Invalid,
                "This project selects a text provider that this version of Wobu does not have.",
            )
            .with_detail(id.to_owned()));
        }
    };
    // Constructing one only fails when the TLS backend will not start, which is
    // a property of the machine and reads as one.
    built.map_err(|e| {
        WobuError::new(Code::ProviderUnavailable, "This provider could not be started.")
            .with_detail(e.to_string())
    })
}

fn label_of(id: &str) -> &str {
    match id {
        anthropic::ID => anthropic::LABEL,
        gemini::ID => gemini::LABEL,
        // A project naming something this build has never heard of is still
        // worth quoting back by name.
        other => other,
    }
}

/* ── the job ──────────────────────────────────────────────────────────────── */

/// Where `enhance:delta` goes.
///
/// A boxed closure rather than an `AppHandle`, so that the half of this module
/// that actually matters — read a stream, validate it, decide what it cost — can
/// be driven by a test without a running Tauri app. The real one is three lines
/// in `enhance_start`.
type Emit = Arc<dyn Fn(EnhanceDelta) + Send + Sync>;

struct EnhanceTask {
    node_id: Id,
    kind: NodeKind,
    label: String,
    project: Id,
    request: EnhanceRequest,
    sources: Vec<Id>,
    provider: Arc<dyn TextProvider>,
    pending: Pending,
    emit: Emit,
}

#[async_trait]
impl Task for EnhanceTask {
    fn kind(&self) -> JobKind {
        JobKind::Enhance
    }

    fn subject_id(&self) -> Option<String> {
        Some(self.node_id.to_string())
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    /// The cancellation token is handed straight to the adapter, which races
    /// every read against it and drops the response body when it loses. That is
    /// what makes Stop stop the *provider* rather than stop us listening, and it
    /// is why there is nothing else here: both adapters already do it, and a
    /// second mechanism would be a second thing to get wrong.
    async fn run(&mut self, ctx: &JobContext) -> Outcome {
        self.attempt(ctx.id(), ctx.cancel()).await
    }
}

impl EnhanceTask {
    async fn attempt(&self, job: JobId, cancel: &Cancel) -> Outcome {
        // Per attempt, not per task. A retry that appended to the previous
        // attempt's buffer would parse as one document with everything written
        // twice, and the editor would show it.
        let mut streamed = String::new();
        let mut next_frame = Instant::now();

        let outcome = {
            let mut sink = |json: &str| {
                streamed.push_str(json);
                let now = Instant::now();
                if now < next_frame {
                    return;
                }
                next_frame = now + FRAME;
                (self.emit)(self.delta(job, &streamed));
            };
            self.provider.enhance(&self.request, &mut sink, cancel).await
        };
        // One last frame with the buffer whole, because the coalescing above
        // will have swallowed whatever arrived inside the final forty
        // milliseconds — which is the end of the document, and the part the user
        // is looking at when it stops moving.
        (self.emit)(self.delta(job, &streamed));

        let usage = outcome.usage;
        match outcome.result {
            Ok(validated) => {
                if !validated.extra_sections.is_empty() {
                    // Not a failure — `wobu-llm` drops them and says so — but a
                    // provider that has started inventing fields should show up
                    // somewhere other than a shrug.
                    diag::info(format!(
                        "{} volunteered sections {} does not declare: {}",
                        self.provider.label(),
                        self.kind,
                        validated.extra_sections.join(", "),
                    ));
                }
                self.pending.remember(Ready {
                    job,
                    project: self.project,
                    node_id: self.node_id,
                    description: validated.description.clone(),
                    questions: validated.questions.clone(),
                    sources: self.sources.clone(),
                });
                Outcome::done_with(EnhanceReady {
                    job_id: job,
                    node_id: self.node_id,
                    description: validated.description,
                    questions: validated.questions,
                })
            }
            // A cancellation is not a failure and must never be retried; the
            // queue holds the same opinion and would override us anyway.
            Err(ProviderError::Cancelled) => Outcome::Cancelled,
            Err(error) => Outcome::failed(failure(&error, usage)),
        }
    }

    /// The buffer so far, as something the editor can draw.
    fn delta(&self, job: JobId, streamed: &str) -> EnhanceDelta {
        let mut sections = Vec::new();
        let mut questions = Vec::new();
        for (key, value) in read_partial(streamed) {
            if key == QUESTIONS_KEY {
                if let SectionValue::List(items) = value {
                    questions = items;
                }
                continue;
            }
            sections.push((key, value));
        }
        EnhanceDelta {
            job_id: job,
            node_id: self.node_id,
            // Normalised so the pane reads in the kind's declared order whatever
            // order the model wrote in, and so a key that is not a section of
            // this kind never reaches the editor at all.
            description: Description::from_sections(sections).normalised_for(self.kind),
            questions,
        }
    }
}

/// A provider failure, in the terms the queue reasons about.
///
/// `from_provider` takes the usage as well as the error, and that pairing is the
/// whole of the retry rule: a stream that died after eight hundred tokens is
/// `Unavailable`, which reads as a free transport blip and is nothing of the
/// sort. Passing the figures means a truncated or malformed response comes back
/// as `retryHeld` — an offer to spend again — rather than being retried on the
/// user's card without asking.
fn failure(error: &ProviderError, usage: Usage) -> Failure {
    let failure = Failure::from_provider(error, usage);
    if usage.total_tokens() == 0 {
        return failure;
    }
    // The queue cannot price a call, so when the user is about to be asked
    // whether to spend again, this is the only thing on screen that says what
    // "again" means.
    failure.cost_note(match usage.cached_input_tokens {
        0 => format!("{} in + {} out", usage.input_tokens, usage.output_tokens),
        cached => {
            format!("{} in ({cached} cached) + {} out", usage.input_tokens, usage.output_tokens,)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use serde_json::json;
    use wobu_llm::{DeltaSink, EnhanceOutcome};

    /// A one-thread executor, the same shape `wobu-llm`'s tests use. The shell
    /// has no async test harness and adding one to run four tests would be a
    /// dependency bought with very little.
    fn block_on<F: Future>(future: F) -> F::Output {
        struct Unparker(std::thread::Thread);
        impl Wake for Unparker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }

    /// How the fake's call ends. The same axis as `wobu-llm`'s fake provider,
    /// because these are the endings a real one has.
    #[derive(Clone, Copy, PartialEq)]
    enum Ending {
        Whole,
        /// A `max_tokens` stop: most of a document arrived and was paid for.
        CutShort,
    }

    struct FakeProvider {
        ending: Ending,
        chunks_sent: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TextProvider for FakeProvider {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn label(&self) -> &'static str {
            "Fake"
        }
        fn default_model(&self) -> &'static str {
            "fake-1"
        }

        async fn enhance(
            &self,
            request: &EnhanceRequest,
            deltas: &mut dyn DeltaSink,
            cancel: &Cancel,
        ) -> EnhanceOutcome {
            let mut usage = Usage { input_tokens: 812, ..Usage::default() };
            let document = document_matching(request.schema());
            let chunks: Vec<String> = document
                .chars()
                .collect::<Vec<_>>()
                .chunks(9)
                .map(|c| c.iter().collect())
                .collect();
            let deliver = match self.ending {
                Ending::Whole => chunks.len(),
                Ending::CutShort => chunks.len() * 2 / 3,
            };

            let mut streamed = String::new();
            for chunk in chunks.iter().take(deliver) {
                if cancel.is_cancelled() {
                    return EnhanceOutcome::new(usage, Err(ProviderError::Cancelled));
                }
                deltas.delta(chunk);
                streamed.push_str(chunk);
                usage.output_tokens += 1;
                self.chunks_sent.fetch_add(1, Ordering::SeqCst);
            }

            let result = match self.ending {
                Ending::Whole => wobu_llm::parse_description(request.kind, &streamed),
                // Reported without being handed to the validator: a truncated
                // document that happened to parse would otherwise be written.
                Ending::CutShort => Err(ProviderError::Truncated),
            };
            EnhanceOutcome::new(usage, result)
        }
    }

    /// A response built from the schema it was handed, so a registry the
    /// validator would reject fails here rather than on a paid call.
    fn document_matching(schema: Value) -> String {
        let mut object = Map::new();
        for (key, property) in schema["properties"].as_object().unwrap() {
            let value = match property["type"].as_str().unwrap() {
                "string" => json!("Ash-glazed ceramic plate over oiled leather."),
                "array" if property["items"]["pattern"].is_string() => {
                    json!(["#2b2118", "#c2703a"])
                }
                "array" if key == QUESTIONS_KEY => json!(["What is on the guild signet?"]),
                _ => json!(["Ember-lit throat vents"]),
            };
            object.insert(key.clone(), value);
        }
        serde_json::to_string(&Value::Object(object)).unwrap()
    }

    /// A task over a fake provider, with the deltas collected instead of
    /// emitted. Returns the task, the frames it painted, and how many chunks the
    /// provider produced — which is how "the work stopped" is told apart from
    /// "the deltas stopped".
    fn task(ending: Ending) -> (EnhanceTask, Arc<Mutex<Vec<EnhanceDelta>>>, Arc<AtomicUsize>) {
        let frames: Arc<Mutex<Vec<EnhanceDelta>>> = Arc::default();
        let seen = Arc::clone(&frames);
        let chunks_sent = Arc::new(AtomicUsize::new(0));
        let task = EnhanceTask {
            node_id: wobu_core::new_id(),
            kind: NodeKind::Character,
            label: "Enhance Kael Vantris".into(),
            project: wobu_core::new_id(),
            request: EnhanceRequest::new(NodeKind::Character, "fake-1", "Describe Kael.")
                .with_system(SYSTEM),
            sources: vec![wobu_core::new_id()],
            provider: Arc::new(FakeProvider { ending, chunks_sent: Arc::clone(&chunks_sent) }),
            pending: Pending::default(),
            emit: Arc::new(move |delta| seen.lock().push(delta)),
        };
        (task, frames, chunks_sent)
    }

    #[test]
    fn a_finished_call_leaves_a_description_waiting_rather_than_writing_one() {
        // Step 4 of the pipeline is "on accept", and this is the half before it.
        // The task holds no project handle at all, so there is no path from here
        // to a node file — a description becomes canon only when somebody says
        // so, through `enhance_accept`.
        let (task, _, _) = task(Ending::Whole);
        let job = JobId::new();

        let Outcome::Done(Some(ready)) = block_on(task.attempt(job, &Cancel::new())) else {
            panic!("a whole response should finish");
        };

        let waiting = task.pending.get(job).expect("the description is waiting");
        assert_eq!(waiting.node_id, task.node_id);
        assert_eq!(
            waiting.sources, task.sources,
            "the stamp is made of the walk it was built from"
        );
        assert!(waiting.description.sections.contains_key("never"));

        // And the questions rode out beside the description rather than inside
        // it, so nothing ever asks an image model what is on the guild signet.
        assert_eq!(ready["questions"][0], "What is on the guild signet?");
        assert!(ready["description"]["sections"].get(QUESTIONS_KEY).is_none(), "{ready}");
        assert!(!waiting.description.sections.contains_key(QUESTIONS_KEY));
        assert_eq!(ready["nodeId"], task.node_id.to_string());
        assert_eq!(ready["jobId"], job.to_string());
    }

    #[test]
    fn a_partial_stream_reaches_the_editor_and_never_the_waiting_room() {
        // The rule this module is built around. Most of a document arrived, the
        // pane drew it, and the provider billed for every token of it — and none
        // of that makes it a description. The only route to one is
        // `EnhanceOutcome::result`, and a truncation is not `Ok`.
        let (task, frames, _) = task(Ending::CutShort);
        let job = JobId::new();

        let outcome = block_on(task.attempt(job, &Cancel::new()));

        assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
        assert!(task.pending.get(job).is_none(), "half a description was left to be accepted");
        let painted = frames.lock();
        assert!(painted.len() > 1, "the editor was shown nothing");
        assert!(
            painted.last().unwrap().description.sections.values().any(|v| !v.is_empty()),
            "the pane should still be showing what did arrive",
        );
    }

    #[test]
    fn the_last_frame_is_the_whole_document_however_the_repaints_were_coalesced() {
        // The frames are throttled, so whatever arrives inside the last forty
        // milliseconds — which is the end of the description — would be
        // swallowed without the final emit. A pane that stops one sentence short
        // and stays there looks exactly like a truncated response.
        let (task, frames, _) = task(Ending::Whole);
        let job = JobId::new();

        let outcome = block_on(task.attempt(job, &Cancel::new()));
        assert!(matches!(outcome, Outcome::Done(_)));

        let painted = frames.lock();
        let last = painted.last().expect("at least one frame");
        let waiting = task.pending.get(job).unwrap();
        assert_eq!(
            last.description, waiting.description,
            "the editor and the node disagree about what was written",
        );
        assert_eq!(last.questions, ["What is on the guild signet?"]);
        assert_eq!(last.job_id, job);
        assert_eq!(last.node_id, task.node_id);
    }

    #[test]
    fn stopping_an_enhance_is_a_cancellation_rather_than_a_failure() {
        // A cancellation reported as a failure would be run through the retry
        // rules, and a retryable one would start a fresh paid attempt for
        // somebody who pressed Stop.
        let (task, _, chunks) = task(Ending::Whole);
        let cancel = Cancel::new();
        cancel.cancel();

        let outcome = block_on(task.attempt(JobId::new(), &cancel));

        assert!(matches!(outcome, Outcome::Cancelled), "{outcome:?}");
        assert_eq!(chunks.load(Ordering::SeqCst), 0, "the provider was asked anyway");
    }

    #[test]
    fn a_response_that_was_paid_for_and_came_back_broken_is_handed_up_not_repeated() {
        // The queue's rule, exercised through the failure this task actually
        // produces. `Truncated` is retryable — it could work — but the provider
        // billed for every token it generated, so the decision to spend again
        // belongs to the person paying. Reporting `Billed::Nothing` here is what
        // would quietly take it away from them.
        let (task, _, _) = task(Ending::CutShort);

        let Outcome::Failed(failed) = block_on(task.attempt(JobId::new(), &Cancel::new())) else {
            panic!("a truncated response should fail");
        };

        assert_eq!(failed.code, "provider.bad_response");
        assert!(failed.retryable);
        assert_eq!(failed.billed, wobu_jobs::Billed::Charged);
        assert!(failed.cost_note.is_some(), "the offer has to say what again would cost");
        assert_eq!(
            wobu_jobs::decide(
                &wobu_jobs::RetryPolicy::default(),
                wobu_jobs::Attempts { total: 1, paid: 0 },
                &failed,
            ),
            wobu_jobs::Verdict::Hold,
        );
    }

    #[test]
    fn a_failure_that_cost_nothing_says_so_and_is_retried_for_free() {
        // The other half: a rate limit generated nothing, so the queue should
        // ride it out on its own rather than putting a dialog in front of a blip.
        let free = failure(
            &ProviderError::RateLimited { provider: "Anthropic", retry_after: None },
            Usage::default(),
        );
        assert_eq!(free.billed, wobu_jobs::Billed::Nothing);
        assert!(free.cost_note.is_none(), "nothing was spent, so there is nothing to quote");
        assert!(matches!(
            wobu_jobs::decide(
                &wobu_jobs::RetryPolicy::default(),
                wobu_jobs::Attempts { total: 1, paid: 0 },
                &free,
            ),
            wobu_jobs::Verdict::Free(_),
        ));
    }

    #[test]
    fn an_answered_description_is_forgotten_and_an_old_unanswered_one_is_dropped() {
        // Nothing but an answer clears this, so without the bound a long session
        // would hold every description it ever produced.
        let pending = Pending::default();
        let ready = |job| Ready {
            job,
            project: wobu_core::new_id(),
            node_id: wobu_core::new_id(),
            description: Description::default(),
            questions: vec![],
            sources: vec![],
        };

        let first = JobId::new();
        pending.remember(ready(first));
        assert!(pending.get(first).is_some());
        // Read, not taken: a `RefusedEdit` is answered by calling accept again
        // with `force`, and there is no second copy of the answer anywhere.
        assert!(pending.get(first).is_some(), "reading it consumed it");
        pending.forget(first);
        assert!(pending.get(first).is_none());

        let mut ids = Vec::new();
        for _ in 0..KEPT + 3 {
            let id = JobId::new();
            pending.remember(ready(id));
            ids.push(id);
        }
        assert!(pending.get(ids[0]).is_none(), "the oldest should have been dropped");
        assert!(pending.get(*ids.last().unwrap()).is_some(), "the newest is still there");

        pending.clear();
        assert!(pending.get(*ids.last().unwrap()).is_none());
    }

    #[test]
    fn a_reloaded_pane_can_ask_for_the_answer_this_process_is_still_holding() {
        // The one failure in this pipeline that costs money to recover from. A
        // webview reload loses the `job:done` that carried the description, and
        // without a way to ask for it again the only way back is running the
        // call — and paying for it — a second time.
        let pending = Pending::default();
        let (ashfall, other) = (wobu_core::new_id(), wobu_core::new_id());
        let waiting = |project, questions: &[&str]| Ready {
            job: JobId::new(),
            project,
            node_id: wobu_core::new_id(),
            description: Description::from_sections([(
                "silhouette".to_string(),
                SectionValue::Text("Tall, narrow, hooded".into()),
            )]),
            questions: questions.iter().map(|q| (*q).to_string()).collect(),
            sources: vec![wobu_core::new_id()],
        };

        let first = waiting(ashfall, &["What is on the guild signet?"]);
        let (job, node) = (first.job, first.node_id);
        pending.remember(first);
        pending.remember(waiting(ashfall, &[]));
        pending.remember(waiting(other, &[]));

        let listed = pending.list(ashfall);
        assert_eq!(listed.len(), 2, "the other project's description is not this one's");
        let found = listed.iter().find(|r| r.node_id == node).expect("matched by node");
        // Both halves come back, because losing the questions would leave the
        // user re-running the call to find out what the model could not settle.
        assert_eq!(found.description.text("silhouette"), Some("Tall, narrow, hooded"));
        assert_eq!(found.questions, ["What is on the guild signet?"]);
        // And the id to answer with, which a reloaded pane no longer has.
        assert_eq!(found.job_id, job);

        // Reading the list does not consume it — the pane may reload twice.
        assert_eq!(pending.list(ashfall).len(), 2);
        assert!(pending.list(wobu_core::new_id()).is_empty(), "a world nobody is in");
    }

    #[test]
    fn a_pending_entry_matches_the_enhanceready_interface() {
        // The same shape rides `job:done` and comes back from `enhance_pending`,
        // so the pane renders one component either way. A rename noticed by
        // neither side arrives as `undefined`.
        let pending = Pending::default();
        let project = wobu_core::new_id();
        pending.remember(Ready {
            job: JobId::new(),
            project,
            node_id: wobu_core::new_id(),
            description: Description::default(),
            questions: vec!["What is on the guild signet?".into()],
            sources: vec![],
        });

        let json = serde_json::to_value(pending.list(project)).unwrap();
        for key in ["jobId", "nodeId", "description", "questions"] {
            assert!(json[0].get(key).is_some(), "`{key}` is missing from EnhanceReady");
        }
        assert!(json[0]["jobId"].is_string(), "a job id must cross as a string");
    }

    /* ── the provider selection ───────────────────────────────────────────── */

    #[test]
    fn a_project_that_names_a_provider_is_not_quietly_given_a_different_one() {
        // The regression worth having: hardcoding Anthropic here would bill the
        // wrong vendor, read the wrong keychain entry, and be invisible until
        // somebody wondered why their Gemini key was never used.
        let providers = json!({
            "text": { "provider": "gemini", "model": "gemini-3.6-flash" },
            "image": { "provider": "comfyui" },
        });
        assert_eq!(
            selection(providers.as_object().unwrap()),
            Selection { provider: "gemini".into(), model: Some("gemini-3.6-flash".into()) }
        );
    }

    #[test]
    fn a_project_that_names_nothing_gets_the_default_text_provider_and_no_model() {
        // Every project created before there is a settings pane for this is in
        // this state, and Enhance has to work in it. An absent model is the
        // adapter's own default rather than a string spelled out here, because
        // model ids move faster than anything else in `docs/08-providers.md`.
        for empty in [
            json!({}),
            json!({ "image": { "provider": "comfyui" } }),
            json!({
                "text": { "model": "  " }
            }),
        ] {
            let selection = selection(empty.as_object().unwrap());
            assert_eq!(selection.provider, anthropic::ID);
            assert_eq!(selection.model, None, "{empty}");
        }
    }

    #[test]
    fn every_adapter_this_build_has_can_be_built_and_named_without_a_network() {
        // Both halves of the two-table split: a provider that can be constructed
        // but not named would leave the "no key on this machine" message saying
        // nothing useful, and one that can be named but not constructed would
        // fail after the key had already been read out of the keychain.
        for id in [anthropic::ID, gemini::ID] {
            let provider = text_provider(id, &Secret::new("not-a-real-key")).unwrap();
            assert_eq!(provider.id(), id);
            assert_eq!(label_of(id), provider.label());
            assert!(!provider.default_model().is_empty());
        }

        let Err(unknown) = text_provider("openai", &Secret::new("k")) else {
            panic!("a provider this build does not have cannot be built");
        };
        assert_eq!(serde_json::to_value(&unknown).unwrap()["code"], "node.invalid");
        assert!(no_key("gemini").message.contains("Gemini"), "the message names the vendor");
        assert_eq!(
            serde_json::to_value(no_key("gemini")).unwrap()["retryable"],
            false,
            "trying again without pasting a key fails identically",
        );
    }

    /* ── the bridge ───────────────────────────────────────────────────────── */

    #[test]
    fn a_delta_matches_the_enhancedelta_interface() {
        // Hand-written TypeScript on the far side, so a serde rename nothing
        // noticed arrives in the pane as `undefined` rather than as an error.
        let (task, _, _) = task(Ending::Whole);
        let delta = task.delta(JobId::new(), r#"{"silhouette":"Tall, narrow-should"#);
        let json = serde_json::to_value(&delta).unwrap();

        for key in ["jobId", "nodeId", "description", "questions"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from EnhanceDelta");
        }
        assert!(json["jobId"].is_string(), "a job id must cross as a string");
        // The same tagged `SectionValue` shape a node's description crosses in,
        // so the pane renders a half-written description with the component it
        // already has rather than a second one.
        assert_eq!(json["description"]["sections"]["silhouette"]["type"], "text");
        assert_eq!(json["description"]["sections"]["silhouette"]["value"], "Tall, narrow-should",);
    }

    #[test]
    fn a_refused_edit_reaches_the_webview_as_the_question_it_is() {
        // `edited` must never be silently overwritten, and the shape is how the
        // UI can tell "your hand-written description is about to go" from
        // "something failed". A failure is what an `Err` here would look like,
        // and the answer to it would be a dismissed dialog rather than a choice.
        let node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
        let refused = Accepted::RefusedEdit { node: Box::new(node.clone()) };
        let json = serde_json::to_value(&refused).unwrap();

        assert_eq!(json["outcome"], "refusedEdit");
        assert_eq!(json["node"]["name"], "Kael Vantris");
        assert_eq!(
            serde_json::to_value(Accepted::Saved { node: Box::new(node) }).unwrap()["outcome"],
            "saved",
        );
    }

    #[test]
    fn a_job_id_survives_the_round_trip_the_webview_puts_it_through() {
        // `enhance_start` returns a string and `enhance_accept` takes one back,
        // because that is what JSON has. An id that could not be parsed back
        // would make every description unacceptable.
        let id = JobId::new();
        assert_eq!(job_id_of(&id.to_string()).unwrap(), id);
        assert_eq!(
            serde_json::to_value(job_id_of("not-a-job").unwrap_err()).unwrap()["code"],
            "internal",
        );
    }
}
