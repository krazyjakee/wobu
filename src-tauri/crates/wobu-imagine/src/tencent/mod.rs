//! Tencent Hunyuan3D: the mesh backend.
//!
//! The third adapter and the one shaped least like the others. `comfy/` is the
//! closest analogue — both submit work and then watch it — but ComfyUI pushes
//! events down a websocket and this one has to ask, over and over, for minutes,
//! against an endpoint that charges for the job whether or not anybody is still
//! waiting.
//!
//! Five things this adapter does that a thinner one would not, each of them a day
//! somebody has already lost:
//!
//! 1. **It calls exactly one namespace.** Three overlapping ones exist, the one
//!    with 3.1 on it has the oldest-looking version string, and the wrong choice
//!    answers `ResourceUnavailable.InterfaceNotExist` — which reads like a broken
//!    action rather than a wrong host. `endpoint.rs` holds the three constants and
//!    there is no setter for any of them.
//! 2. **It polls the region it submitted to.** Not by remembering to: the submit
//!    returns a [`JobTicket`] that owns its [`Endpoint`], and the poll builds its
//!    call from the ticket. A job submitted to Singapore and polled in Frankfurt
//!    is not an error, it is `FailedOperation.JobNotFound` — the same answer as a
//!    job that never existed, on a call that was billed.
//! 3. **It downloads on `DONE` and hands back bytes.** Result URLs are valid for
//!    24 hours, so a stored one works for the whole of an afternoon's testing and
//!    is dead by morning. [`GeneratedMesh`] has no field to put one in.
//! 4. **It unzips.** The OBJ result URL is a `.zip` of the mesh, its `.mtl` and
//!    its texture maps — see `archive.rs` — and those bytes written to `model.obj`
//!    make a file every viewer refuses.
//! 5. **It decides what a download is from the bytes, not from `Type`.** The
//!    international documentation contradicts itself about which types are
//!    returned, so `Type` orders the candidates and nothing else.
//!
//! ## What is not here, and where it is instead
//!
//! **The three-concurrent-job cap.** `docs/08-providers.md` records a limit of
//! three concurrent Pro jobs and 20 requests a second, and both belong to
//! `wobu-jobs`: `queue.rs` already defaults `concurrency` to 3 and names Hunyuan3D
//! as the reason. A semaphore in this file would be a second admission controller
//! that the queue could not see, that `Queue::set_concurrency` could not move, and
//! that would be counting a per-*account* limit from inside a per-*backend*
//! object — two projects open, two backends, six jobs. What this file owns is one
//! job, honestly reported and stoppable.
//!
//! **A crash-resumable job.** A `JobId` outlives our process by up to 24 hours, so
//! resuming one after a restart would save real money. It is deliberately not done
//! here: [`JobTicket`] is not serialisable, because a resumable-looking record with
//! an expiry nobody checks is worse than no record, and the decision about what
//! `wobu-jobs` persists across a restart is not this adapter's to make by accident.
//!
//! ## What has never run
//!
//! **All of it.** There are no Tencent credentials in this tree, and every
//! statement below about what the provider answers comes from
//! `docs/08-providers.md` — whose region and namespace findings *were* verified
//! against a live account on 2026-07-31, and whose parameter shapes were not.
//! `wire.rs` marks each unverified field. The signing is the one part with real
//! vectors behind it, and they are Tencent's own; see `sign.rs`.

mod archive;
mod endpoint;
mod sign;
mod wire;

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use crate::Cancel;
use crate::backend::ProgressSink;
use crate::error::{Error, Result};
use crate::mesh::{
    FACE_COUNT, GenerateType, GeneratedMesh, MeshBackend, MeshCapabilities, MeshFile, MeshFormat,
    MeshOutcome, MeshRequest, MeshUsage,
};

use endpoint::{Endpoint, JobTicket};
use wire::{Progress, ResultFile, Status};

pub use endpoint::{JOB_ID_LIFETIME, Region};
pub use sign::{CONTENT_TYPE, Call, Credentials, SecretKey, Signed, auth_failure, sign};

/// The `backend` in `project.json`, the `backend` field of every `Generation`,
/// and the `wobu/hunyuan3d` entry in the OS keychain — which holds a `SecretId`
/// and a `SecretKey` rather than one string, because this provider has no bearer
/// key at all.
pub const ID: &str = "hunyuan3d";

/// The name a person sees, including inside every error built here.
pub const LABEL: &str = sign::BACKEND;

/// Used when a project names this backend but no model.
///
/// **`3.1` is a parameter and not an endpoint**, and the provider's own default
/// is `3.0` — so an omitted `Model` is a silently older reconstruction at the same
/// price. Defaulting to 3.1 here is what makes multi-view input work, which is the
/// whole reason the Turnaround preset emits eight named views.
pub const DEFAULT_MODEL: &str = "3.1";

/// The model the multi-view path needs. Named rather than compared against a
/// literal in three places.
const MODEL_3_1: &str = "3.1";
const MODEL_3_0: &str = "3.0";

/// Long enough for a slow network, short enough that a black hole is not mistaken
/// for a slow provider. A connect timeout only: a submit that has gone out is
/// billed, and a whole-request timeout would abandon a job we would then never
/// poll.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How the poll paces itself, and when it gives up.
///
/// A value rather than three constants so the tests can drive a whole
/// `WAIT → RUN → DONE` sequence without waiting for any of it — the same trick
/// `comfy/socket.rs` uses to drive a run from recorded frames without a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Schedule {
    first: Duration,
    max: Duration,
    /// Total time spent polling before the job is given up on.
    ///
    /// Must stay well under [`JOB_ID_LIFETIME`] — checked by a test — because a
    /// deadline past it would mean the last poll of a long run asking about a job
    /// id that had expired, and being told the job never existed.
    deadline: Duration,
}

impl Default for Schedule {
    fn default() -> Schedule {
        Schedule {
            // A mesh takes minutes, so the first poll is not urgent. It is still
            // short because `WAIT` is the phase the user is most likely to cancel
            // in, and a cancellation is noticed between polls.
            first: Duration::from_secs(3),
            // Twenty requests a second is the published rate limit and this is
            // nowhere near it. The ceiling exists so a job that runs long does not
            // send a thousand polls.
            max: Duration::from_secs(15),
            // Generous. `docs/08-providers.md` publishes no upper bound on how
            // long a Pro job takes, and giving up on a job that was going to
            // finish costs the whole of what it cost to run.
            deadline: Duration::from_secs(30 * 60),
        }
    }
}

impl Schedule {
    /// Doubling, capped. Attempt zero is the wait before the first poll.
    fn delay(&self, attempt: u32) -> Duration {
        self.first.saturating_mul(2u32.saturating_pow(attempt.min(8))).min(self.max)
    }

    /// Every delay is zero, so a test drives the whole sequence at once.
    #[cfg(test)]
    fn immediate() -> Schedule {
        Schedule {
            first: Duration::ZERO,
            max: Duration::ZERO,
            deadline: Duration::from_secs(30 * 60),
        }
    }
}

/// A Tencent Cloud key pair, a region, and the client that uses them.
///
/// Constructing one does no IO and cannot fail on a machine with no network: the
/// Inspector draws a backend dropdown before anything has been checked, exactly
/// as it does for the other two.
pub struct HunyuanBackend {
    credentials: Credentials,
    endpoint: Endpoint,
    client: reqwest::Client,
    schedule: Schedule,
}

impl fmt::Debug for HunyuanBackend {
    /// Hand-written, and the stakes are higher than the other two adapters':
    /// `Credentials` masks both halves in its own `Debug` because a Tencent
    /// `SecretKey` is an account-wide master credential, and a derived `Debug`
    /// here would print the struct that holds it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HunyuanBackend")
            .field("credentials", &self.credentials)
            .field("region", &self.endpoint.region)
            .finish()
    }
}

impl HunyuanBackend {
    /// **`region` is required and is one of three.** `docs/08-providers.md`: only
    /// `ap-singapore`, `na-siliconvalley` and `eu-frankfurt` are served, and
    /// `ap-guangzhou` — which appears throughout Tencent's own examples — is not.
    /// [`Region::nearest_to_utc_offset`] is the default worth offering, and it is
    /// the caller's to apply because this crate does not read the clock's zone.
    pub fn new(credentials: Credentials, region: Region) -> Result<HunyuanBackend> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            // A client that will not build is a machine problem — a TLS backend
            // that would not start — and reads as one.
            .map_err(|e| Error::Unavailable { detail: e.to_string() })?;
        Ok(HunyuanBackend {
            credentials,
            endpoint: Endpoint::new(region),
            client,
            schedule: Schedule::default(),
        })
    }

    pub fn region(&self) -> Region {
        self.endpoint.region
    }

    /// One mesh, from a request to files we are willing to keep.
    ///
    /// Split out of [`generate`](MeshBackend::generate) so the whole path can be
    /// written with `?`. The wrapper is the only place a usage figure is decided,
    /// and here that matters more than it does for an image: the moment a `JobId`
    /// exists the job is billed, and everything after that point — the polling,
    /// the download, the unzip — can fail without making it free.
    async fn build(
        &self,
        request: &MeshRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> (MeshUsage, Result<GeneratedMesh>) {
        // A job cancelled while it was queued must not submit one. Everything
        // before the submit is unbilled and everything after it is not.
        if cancel.is_cancelled() {
            return (MeshUsage::free(), Err(Error::Cancelled));
        }

        let body = match wire::submit_body(request, &self.capabilities(&request.model)) {
            Ok(body) => body,
            // Refused before signing, so nothing was sent and nothing was
            // charged. These are all `Unsupported`, which is our bug by
            // construction — see `wire.rs`.
            Err(error) => return (MeshUsage::free(), Err(error)),
        };

        progress.step(0, 1, Some("submitting to Tencent"));
        let ticket = match self.submit(&body, cancel).await {
            Ok(ticket) => ticket,
            Err(error) => return (MeshUsage::free(), Err(error)),
        };

        // From here on the job is paid for whatever happens, including a
        // cancellation: `docs/08-providers.md` gives no cancel action, so pressing
        // Stop stops us waiting and does not stop them generating.
        let billed = MeshUsage::billed(1);
        let files = match self.wait(&ticket, progress, cancel).await {
            Ok(files) => files,
            Err(error) => return (billed, Err(error)),
        };
        (billed, self.collect(&files, cancel).await)
    }

    /// Start a job, and bind its id to the endpoint that accepted it.
    async fn submit(&self, body: &str, cancel: &Cancel) -> Result<JobTicket> {
        let answer = self.send(&self.endpoint.call(endpoint::SUBMIT, body), cancel).await?;
        Ok(JobTicket::new(wire::submitted(&answer)?, self.endpoint))
    }

    /// Ask about a job, at the endpoint the ticket carries.
    ///
    /// The call is built by [`poll_call`], which takes the ticket and no `&self`
    /// — so there is no backend region *in scope* to reach for by mistake, rather
    /// than one that has to be remembered not to use.
    async fn query(&self, ticket: &JobTicket, cancel: &Cancel) -> Result<Progress> {
        let body = wire::query_body(ticket.job_id());
        let answer = self.send(&poll_call(ticket, &body), cancel).await?;
        wire::progress(&answer)
    }

    /// Poll until the job finishes, fails, is cancelled, or runs out of patience.
    async fn wait(
        &self,
        ticket: &JobTicket,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> Result<Vec<ResultFile>> {
        watch(self.schedule, || self.query(ticket, cancel), progress, cancel).await
    }

    /// Download the best candidate that turns out to be a mesh, and everything it
    /// references.
    ///
    /// Candidates are tried in order rather than the first being trusted, because
    /// what a `Type` claims and what the bytes are do not have to agree — and a
    /// job that has already been paid for is worth a second GET before it is
    /// declared a failure. A *download* that fails is not retried against the next
    /// candidate: an expired signature or a dead network is the same answer for
    /// every URL in the list, so trying them all would be three requests to learn
    /// one thing.
    async fn collect(&self, files: &[ResultFile], cancel: &Cancel) -> Result<GeneratedMesh> {
        let candidates = wire::candidates(files);
        if candidates.is_empty() {
            return Err(Error::NoMesh);
        }
        let mut last = None;
        for candidate in &candidates {
            let bytes = self.download(&candidate.url, cancel).await?;
            match assemble(bytes) {
                Ok((format, mesh, extras)) => {
                    // Fetched last and separately: `PreviewImageUrl` expires on
                    // the same 24-hour clock as the mesh, so it is bytes now or it
                    // is nothing later. A preview we cannot fetch is not worth
                    // failing a finished mesh over.
                    let preview = match &candidate.preview {
                        Some(url) => self
                            .download(url, cancel)
                            .await
                            .ok()
                            .map(|bytes| MeshFile::new(preview_name(url), bytes)),
                        None => None,
                    };
                    return Ok(GeneratedMesh { format, mesh, extras, preview });
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => last = Some((candidate.kind.clone(), error)),
            }
        }
        Err(match last {
            Some((kind, Error::NotAMesh { detail })) => Error::NotAMesh {
                detail: format!("the `{kind}` result is not something wobu can open — {detail}"),
            },
            Some((_, error)) => error,
            None => Error::NoMesh,
        })
    }

    /// Fetch a result URL. Unsigned — these point at object storage rather than
    /// at the API, and they carry their own authorisation in the query string,
    /// which is the other reason they expire.
    async fn download(&self, url: &str, cancel: &Cancel) -> Result<Vec<u8>> {
        let response = match until_cancelled(self.client.get(url).send(), cancel).await {
            None => return Err(Error::Cancelled),
            Some(Err(e)) => return Err(unreachable(&e)),
            Some(Ok(response)) => response,
        };
        let status = response.status().as_u16();
        let body = match until_cancelled(response.bytes(), cancel).await {
            None => return Err(Error::Cancelled),
            Some(body) => body.map(|bytes| bytes.to_vec()).unwrap_or_default(),
        };
        match status {
            200..=299 => Ok(body),
            // The expiry, seen from the download rather than from the API. Object
            // storage answers an expired signature with a 403 and an XML body, so
            // this is the one place the 24-hour clock shows up as a status code.
            403 => Err(Error::Unavailable {
                detail: format!(
                    "{LABEL} would not serve the finished mesh — a result URL is only valid for \
                     24 hours, and this one is no longer accepted"
                ),
            }),
            _ => Err(Error::Unavailable {
                detail: format!("{LABEL} answered HTTP {status} for the finished mesh"),
            }),
        }
    }

    /// One signed API call.
    ///
    /// The body is written to the socket **exactly** as it was signed. Not
    /// `RequestBuilder::json`, which would re-serialise it and set a
    /// `Content-Type` of its own — `sign.rs` warns about precisely that, because
    /// the result is an `AuthFailure.SignatureFailure` that reads as a bad key and
    /// sends the user to regenerate an account-wide master credential.
    async fn send(&self, call: &Call<'_>, cancel: &Cancel) -> Result<Vec<u8>> {
        let signed = sign(call, &self.credentials, now());
        // The URL is built from the call that was signed rather than from a
        // constant read separately, so the host the signature covers and the host
        // the request goes to cannot drift apart.
        let mut request =
            self.client.post(format!("https://{}/", call.host)).body(call.body.to_owned());
        for (name, value) in signed.headers() {
            request = request.header(name, value);
        }

        let response = match until_cancelled(request.send(), cancel).await {
            None => return Err(Error::Cancelled),
            Some(Err(e)) => return Err(unreachable(&e)),
            Some(Ok(response)) => response,
        };
        let status = response.status().as_u16();
        let answer = match until_cancelled(response.bytes(), cancel).await {
            None => return Err(Error::Cancelled),
            Some(body) => body.map(|bytes| bytes.to_vec()).unwrap_or_default(),
        };
        match status {
            // Including every application failure: Tencent reports those inside a
            // 200 with an `Error.Code` in the body, which `wire.rs` reads. A
            // reader that switched on the status first would see a successful
            // submit with no job id.
            200..=299 => Ok(answer),
            _ => Err(Error::Unavailable {
                detail: format!(
                    "{LABEL} answered HTTP {status}, which is a transport failure rather than \
                     one of its own — the API reports its errors inside a 200"
                ),
            }),
        }
    }
}

#[async_trait]
impl MeshBackend for HunyuanBackend {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        LABEL
    }

    fn default_model(&self) -> &'static str {
        DEFAULT_MODEL
    }

    /// What this model can do, from `docs/08-providers.md`.
    ///
    /// The two differ in both directions — 3.1 takes twice the views and loses two
    /// generate modes — so one answer for the backend would have to be the worse
    /// of them and would throw away the multi-view input that is the entire reason
    /// to prefer 3.1.
    ///
    /// Total over unknown ids, and the unknown answer is the **intersection**
    /// rather than the union: a project naming a model that has been retired, or
    /// one released next month, gets the capabilities both known models have. The
    /// alternative is offering eight views to something that takes four, which is
    /// a paid call refused after the upload.
    fn capabilities(&self, model: &str) -> MeshCapabilities {
        let (max_views, generate_types) = match model {
            MODEL_3_1 => (8, vec![GenerateType::Normal, GenerateType::Geometry]),
            MODEL_3_0 => (
                4,
                vec![
                    GenerateType::Normal,
                    GenerateType::Geometry,
                    GenerateType::LowPoly,
                    GenerateType::Sketch,
                ],
            ),
            _ => (4, vec![GenerateType::Normal, GenerateType::Geometry]),
        };
        MeshCapabilities {
            max_views,
            face_count: FACE_COUNT,
            pbr: true,
            generate_types,
            text_to_mesh: true,
            // There is no free tier here and no local fallback. `capability.rs`'s
            // meaning applies unchanged: this call costs money.
            requires_billing: true,
        }
    }

    async fn generate(
        &self,
        request: &MeshRequest,
        progress: &mut dyn ProgressSink,
        cancel: &Cancel,
    ) -> MeshOutcome {
        let (usage, result) = self.build(request, progress, cancel).await;
        MeshOutcome::new(usage, result)
    }
}

/// The poll loop, with the querying handed in.
///
/// A free function generic over the query so that a whole `WAIT → RUN → DONE`
/// sequence can be driven from recorded payloads with no server and no waiting —
/// the same split `comfy/socket.rs` makes for the websocket. Everything about the
/// loop that can be wrong is in here: when it gives up, what it does with a status
/// it has never seen, how often it reports, and whether a cancellation is noticed
/// during a wait or only between them.
async fn watch<Q, F>(
    schedule: Schedule,
    mut query: Q,
    progress: &mut dyn ProgressSink,
    cancel: &Cancel,
) -> Result<Vec<ResultFile>>
where
    Q: FnMut() -> F,
    F: Future<Output = Result<Progress>>,
{
    let mut waited = Duration::ZERO;
    let mut attempt = 0u32;
    let mut said: Option<Status> = None;

    loop {
        let delay = schedule.delay(attempt);
        // Raced rather than checked between polls. A delay is seconds long and a
        // job is minutes long, so a loop that only tested the flag between
        // requests would leave a stopped job sitting in a sleep — and the user,
        // who pressed Stop, watching a spinner.
        if until_cancelled(sleep(delay), cancel).await.is_none() {
            return Err(Error::Cancelled);
        }
        waited += delay;
        attempt += 1;

        let found = query().await?;
        // Throttled at the source, as `ProgressSink::step` asks: a thirty-minute
        // job is a hundred and twenty polls and three phases, and the status bar
        // draws the last one.
        if said.as_ref() != Some(&found.status) {
            progress.step(0, 1, Some(&found.status.note()));
            said = Some(found.status.clone());
        }

        match found.status {
            Status::Done => {
                return match found.files.is_empty() {
                    true => Err(Error::NoMesh),
                    false => Ok(found.files),
                };
            }
            Status::Fail => {
                let (code, message) = found.failure.unwrap_or_default();
                return Err(wire::failure(&code, &message));
            }
            // `WAIT`, `RUN`, and anything the provider has renamed or added. An
            // unknown status keeps the poll going rather than failing it, because
            // abandoning a running job costs the whole of what it cost to run —
            // and the deadline below is what stops that being forever.
            _ => {}
        }

        if waited >= schedule.deadline {
            return Err(Error::Unavailable {
                detail: format!(
                    "{LABEL} has been reporting `{}` for {} minutes. The job may still finish — \
                     it is charged either way — but wobu has stopped waiting for it",
                    found.status.note(),
                    waited.as_secs() / 60,
                ),
            });
        }
    }
}

/// The signed call one poll makes.
///
/// **This function is the whole of "the poll targets the same region as the
/// submit".** It takes the ticket and nothing else — no `&self`, so the backend's
/// own region is not in scope and cannot be reached for. The alternative is a
/// method on the backend with two regions available and a comment asking for the
/// right one, and the failure that produces is not an error: a job submitted to
/// Singapore and polled in Frankfurt answers `FailedOperation.JobNotFound`, which
/// is what a job that never existed answers, on a call that has been billed.
fn poll_call<'a>(ticket: &'a JobTicket, body: &'a str) -> Call<'a> {
    ticket.endpoint().call(endpoint::QUERY, body)
}

/// Turn a download into a mesh and its dependencies.
///
/// **Decided from the bytes.** `docs/08-providers.md`: the international docs
/// "list `Type` values that contradict GLB being returned", so the declared type
/// is a hint about which URL to try first and nothing more. What arrives is either
/// an archive, a self-contained container, or something we say we cannot open.
fn assemble(bytes: Vec<u8>) -> Result<(MeshFormat, MeshFile, Vec<MeshFile>)> {
    if archive::is_glb(&bytes) {
        // Named by us rather than after the URL, which for object storage is
        // usually a hash with a signature stapled to it. Nothing references a
        // `.glb` by name — it carries its own textures — so the name is free to
        // choose, which an `.obj` inside an archive is not.
        return Ok((MeshFormat::Glb, MeshFile::new("model.glb", bytes), Vec::new()));
    }
    if !archive::is_zip(&bytes) {
        return Err(Error::NotAMesh {
            detail: format!(
                "{} bytes arrived that are neither a ZIP archive nor a binary glTF{}",
                bytes.len(),
                preview_of(&bytes),
            ),
        });
    }

    let mut files = archive::unpack(&bytes)?;
    // Preference order, and the reason it is not simply "the biggest file": a
    // texture map is routinely larger than the mesh that uses it.
    let ranked = |file: &MeshFile| match MeshFormat::from_filename(&file.name) {
        MeshFormat::Glb => 0,
        MeshFormat::Gltf => 1,
        MeshFormat::Obj => 2,
        MeshFormat::Fbx => 3,
        MeshFormat::Other(_) => 4,
    };
    let Some(at) = files
        .iter()
        .enumerate()
        .filter(|(_, file)| ranked(file) < 4)
        .min_by_key(|(_, file)| ranked(file))
        .map(|(at, _)| at)
    else {
        return Err(Error::NotAMesh {
            detail: format!(
                "the archive holds no mesh, only {}",
                files.iter().map(|file| file.name.as_str()).collect::<Vec<_>>().join(", "),
            ),
        });
    };
    let mesh = files.remove(at);
    Ok((MeshFormat::from_filename(&mesh.name), mesh, files))
}

/// A name for the preview image, taken from the URL's own path.
///
/// The query string is dropped: these are signed object-storage URLs and the
/// signature is most of their length. A path that yields nothing usable falls back
/// to a fixed name rather than an empty one, because this becomes a filename.
fn preview_name(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    match path.rsplit('/').next() {
        Some(name) if name.contains('.') && !name.contains("..") => name.to_owned(),
        _ => "preview.png".to_owned(),
    }
}

/// The first few bytes, printable, for a download that is neither of the two
/// things it should be. Almost always an XML error document from object storage,
/// and the first line of it is the whole diagnosis.
fn preview_of(bytes: &[u8]) -> String {
    let head: String = bytes
        .iter()
        .take(60)
        .map(|byte| match byte.is_ascii_graphic() || *byte == b' ' {
            true => *byte as char,
            false => '.',
        })
        .collect();
    match head.is_empty() {
        true => String::new(),
        false => format!(" — they begin `{head}`"),
    }
}

/// Unreachable, refused, timed out.
///
/// Nothing reached the API, so nothing was charged — which is why this is
/// separated from the errors the API reports about itself.
fn unreachable(error: &reqwest::Error) -> Error {
    let detail = if error.is_timeout() {
        format!("{LABEL} did not answer within {} seconds", CONNECT_TIMEOUT.as_secs(),)
    } else if error.is_connect() {
        format!(
            "could not connect to {LABEL} — check this machine's network and any proxy between \
             it and {}",
            endpoint::HOST,
        )
    } else {
        format!("could not reach {LABEL}: {error}")
    };
    Error::Unavailable { detail }
}

/// Unix seconds, which is what [`sign`] takes.
///
/// The clock is read here and passed in rather than read inside the signing,
/// because a function that reads the clock cannot be checked against a fixed
/// vector — and the signing is the function that most needs to be. A machine
/// whose clock is before the epoch signs with zero and gets
/// `AuthFailure.SignatureExpire`, which is the message about the system clock,
/// which is correct.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// The third copy of this in the crate, and deliberately not shared.
///
/// `comfy/socket.rs` and `gemini/mod.rs` each have one. Sharing it would mean
/// either making `comfy`'s private socket module public or adding a crate-level
/// utility module, and eight lines of `poll_fn` is less than either — the same
/// judgement `gemini` made when it wrote the second one.
async fn until_cancelled<F: Future>(future: F, cancel: &Cancel) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

/// A timer that belongs to no runtime.
///
/// This crate names no runtime — `Cargo.toml` says so in as many words, and its
/// only async dependencies are ones the adapters bring — so there is no
/// `tokio::time` to reach for, and adding one would make that claim false for the
/// sake of a sleep. A thread that parks and then wakes the waker is runtime-
/// agnostic, works under the hand-written `block_on` the tests use, and costs one
/// short-lived thread per poll interval: at most one per in-flight job, which the
/// queue caps at three.
///
/// A zero duration returns immediately without spawning anything, which is what
/// lets the tests drive a whole poll sequence at once.
fn sleep(duration: Duration) -> Sleep {
    Sleep { duration, shared: None }
}

struct Sleep {
    duration: Duration,
    shared: Option<Arc<Timer>>,
}

struct Timer {
    elapsed: AtomicBool,
    /// Refreshed on every poll. A future can be polled by a different task than
    /// the one that polled it first — a `select` that re-registers, an executor
    /// that migrates work — and a thread holding the first waker would then wake
    /// nobody, which is a generate that hangs until the process ends.
    waker: Mutex<Option<Waker>>,
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.duration.is_zero() {
            return Poll::Ready(());
        }
        let shared = match &self.shared {
            Some(shared) => Arc::clone(shared),
            None => {
                let shared =
                    Arc::new(Timer { elapsed: AtomicBool::new(false), waker: Mutex::new(None) });
                let timer = Arc::clone(&shared);
                let duration = self.duration;
                std::thread::spawn(move || {
                    std::thread::sleep(duration);
                    timer.elapsed.store(true, Ordering::Release);
                    if let Some(waker) = timer.waker.lock().ok().and_then(|mut slot| slot.take()) {
                        waker.wake();
                    }
                });
                self.shared = Some(Arc::clone(&shared));
                shared
            }
        };
        if let Ok(mut slot) = shared.waker.lock() {
            *slot = Some(cx.waker().clone());
        }
        // Re-read after storing the waker, or a timer that fired in between would
        // have taken a `None` and left this pending forever.
        match shared.elapsed.load(Ordering::Acquire) {
            true => Poll::Ready(()),
            false => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;
    use std::task::Wake;
    use std::time::Instant;

    use crate::backend::Discard;
    use crate::mesh::{MeshView, View};

    fn block_on<F: Future>(future: F) -> F::Output {
        struct Unparker(std::thread::Thread);
        impl Wake for Unparker {
            fn wake(self: StdArc<Self>) {
                self.0.unpark();
            }
        }

        let waker = std::task::Waker::from(StdArc::new(Unparker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }

    fn backend() -> HunyuanBackend {
        HunyuanBackend::new(
            Credentials::new(
                "AKIDzzzzzzzzzzzz",
                SecretKey::new("Gu5t9xGARNpq86cd98joQYCN3Cozk1qA"),
            ),
            Region::ApSingapore,
        )
        .unwrap()
    }

    /// One poll answer, in the documented wire shape.
    fn poll(status: &str) -> Vec<u8> {
        format!(r#"{{"Response":{{"Status":"{status}","RequestId":"7f"}}}}"#).into_bytes()
    }

    fn done() -> Vec<u8> {
        br#"{"Response":{"Status":"DONE","ErrorCode":"","ErrorMessage":"",
            "ResultFile3Ds":[{"Type":"OBJ","Url":"https://cos.example/a.zip?sign=x",
            "PreviewImageUrl":"https://cos.example/a.png?sign=x"}],"RequestId":"7f"}}"#
            .to_vec()
    }

    /// Drive [`watch`] over a recorded sequence, with no network and no waiting.
    fn watched(answers: Vec<Vec<u8>>, cancel: &Cancel) -> Result<Vec<ResultFile>> {
        let mut answers = answers.into_iter();
        block_on(watch(
            Schedule::immediate(),
            || {
                let next = answers.next();
                async move {
                    match next {
                        Some(body) => wire::progress(&body),
                        None => Err(Error::Unavailable { detail: "the recording ran out".into() }),
                    }
                }
            },
            &mut Discard,
            cancel,
        ))
    }

    #[test]
    fn a_whole_run_polls_through_wait_and_run_and_finishes_on_done() {
        // The documented vocabulary in the order a job sends it. Everything the
        // user sees during a mesh generation is read off these, and there is no
        // websocket to fall back on if one is misread.
        let files =
            watched(vec![poll("WAIT"), poll("WAIT"), poll("RUN"), done()], &Cancel::new()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, "OBJ");
        assert!(files[0].url.starts_with("https://cos.example/a.zip"));
    }

    #[test]
    fn the_status_bar_is_told_once_per_phase_and_not_once_per_poll() {
        // `ProgressSink::step` asks for throttling at the source. A thirty-minute
        // job is a hundred and twenty polls across three phases, and the status
        // bar draws the last one — so a hundred and seventeen of those events are
        // a redraw of a line that did not change.
        struct Counting(Vec<String>);
        impl ProgressSink for Counting {
            fn step(&mut self, _done: u32, _total: u32, note: Option<&str>) {
                self.0.push(note.unwrap_or_default().to_owned());
            }
        }

        let answers =
            vec![poll("WAIT"), poll("WAIT"), poll("WAIT"), poll("RUN"), poll("RUN"), done()];
        let mut sink = Counting(Vec::new());
        let mut answers = answers.into_iter();
        block_on(watch(
            Schedule::immediate(),
            || {
                let next = answers.next().unwrap();
                async move { wire::progress(&next) }
            },
            &mut sink,
            &Cancel::new(),
        ))
        .unwrap();
        assert_eq!(
            sink.0,
            ["queued at Tencent Hunyuan3D", "generating the mesh", "downloading the mesh",]
        );
    }

    #[test]
    fn a_status_the_provider_has_never_sent_before_keeps_the_job_alive() {
        // A renamed or added status is a release of theirs, not a failure of
        // ours. Treating it as terminal abandons a job that is generating and has
        // been paid for; treating it as success returns no mesh. It keeps
        // polling, and the deadline is what bounds that.
        let files =
            watched(vec![poll("QUEUING"), poll("PROCESSING"), done()], &Cancel::new()).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn a_job_that_never_finishes_gives_up_and_says_it_is_still_being_charged_for() {
        // The user is owed the truth here: we have stopped waiting and Tencent has
        // not stopped generating, so the money is spent either way. A bare
        // "timed out" reads as though nothing happened.
        let mut answers = std::iter::repeat_with(|| poll("RUN"));
        let schedule =
            Schedule { first: Duration::ZERO, max: Duration::ZERO, deadline: Duration::ZERO };
        let error = block_on(watch(
            schedule,
            || {
                let next = answers.next().unwrap();
                async move { wire::progress(&next) }
            },
            &mut Discard,
            &Cancel::new(),
        ))
        .unwrap_err();
        assert!(error.to_string().contains("charged either way"), "{error}");
        assert!(error.to_string().contains("generating the mesh"), "{error}");
    }

    #[test]
    fn the_poll_gives_up_long_before_the_job_id_expires() {
        // The regression: somebody raises the deadline to be generous and it
        // passes 24 hours, at which point the last poll of a long run asks about a
        // job id that has expired and is told the job never existed — reported as
        // a vanished job rather than as our own deadline.
        assert!(
            Schedule::default().deadline < JOB_ID_LIFETIME,
            "a poll deadline past the job id's own lifetime cannot report a real answer",
        );
        assert!(Schedule::default().deadline <= JOB_ID_LIFETIME / 4, "and with room to spare");
    }

    #[test]
    fn the_poll_interval_backs_off_and_stays_well_under_the_published_rate_limit() {
        // Twenty requests a second is the published limit and three concurrent
        // jobs is the other. Even at the shortest interval, three jobs polling
        // together is one request a second between them.
        let schedule = Schedule::default();
        assert_eq!(schedule.delay(0), Duration::from_secs(3));
        assert_eq!(schedule.delay(1), Duration::from_secs(6));
        assert_eq!(schedule.delay(9), schedule.max, "and it stops doubling");
        assert!(schedule.delay(0) >= Duration::from_secs(1));
    }

    #[test]
    fn a_failed_job_reports_the_code_the_query_carried_and_not_a_generic_failure() {
        // A `FAIL` arrives on a successful query minutes after the submit was
        // charged, and its `ErrorCode` is the only description of what went wrong.
        let fail = br#"{"Response":{"Status":"FAIL","ErrorCode":"FailedOperation.ImageIllegal",
            "ErrorMessage":"blocked by moderation","RequestId":"7f"}}"#;
        let error = watched(vec![poll("RUN"), fail.to_vec()], &Cancel::new()).unwrap_err();
        assert!(matches!(error, Error::Refused { .. }), "{error}");
        assert!(error.to_string().contains("blocked by moderation"), "{error}");
    }

    #[test]
    fn a_done_job_with_an_empty_file_list_is_told_apart_from_a_failed_one() {
        // A silent empty result and a stated failure send the user to two
        // different places — one is a bug report and the other is a prompt to
        // edit. Both have been billed.
        let empty = br#"{"Response":{"Status":"DONE","ResultFile3Ds":[],"RequestId":"7f"}}"#;
        assert!(matches!(watched(vec![empty.to_vec()], &Cancel::new()), Err(Error::NoMesh)));
    }

    #[test]
    fn a_cancellation_is_noticed_during_a_wait_rather_than_at_the_end_of_one() {
        // The failure this guards: a fifteen-second poll interval and a user who
        // pressed Stop, watching a spinner for the rest of it. The token is raced
        // against the sleep, so a real interval is interrupted rather than served
        // out — which is what the timing assertion below is checking.
        let cancel = Cancel::new();
        cancel.cancel();
        let started = Instant::now();
        let outcome = block_on(watch(
            Schedule {
                first: Duration::from_secs(30),
                max: Duration::from_secs(30),
                deadline: Duration::from_secs(60),
            },
            || async { panic!("a cancelled watch must not poll the provider") },
            &mut Discard,
            &cancel,
        ));
        assert!(matches!(outcome, Err(Error::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5), "it waited out the interval");
    }

    #[test]
    fn a_cancelled_job_never_submits_one_and_is_never_billed() {
        // The queue can cancel between admitting a job and starting it. Here the
        // cost is money: a submit that has gone out is charged whatever happens
        // afterwards, and there is no cancel action at the provider.
        let cancel = Cancel::new();
        cancel.cancel();
        let request = MeshRequest::from_prompt(DEFAULT_MODEL, "a wrought iron lantern");
        let outcome = block_on(backend().generate(&request, &mut Discard, &cancel));
        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage, MeshUsage::free());
    }

    #[test]
    fn a_request_we_would_not_send_costs_nothing_and_never_opens_a_connection() {
        // The order matters: the body is built and checked before anything is
        // signed, so every `Unsupported` is unbilled by construction. A backend
        // that submitted first would charge for the round trip that tells it the
        // face count was out of range.
        let request = MeshRequest::from_prompt(DEFAULT_MODEL, "p").with_face_count(10);
        let outcome = block_on(backend().generate(&request, &mut Discard, &Cancel::new()));
        assert_eq!(outcome.usage, MeshUsage::free());
        assert_eq!(outcome.result.unwrap_err().code(), "internal");
    }

    #[test]
    fn a_poll_is_signed_for_the_region_the_submit_used_and_not_the_backends_current_one() {
        // The correctness bug the issue calls out by name. A job submitted to
        // Singapore and polled in Frankfurt is not an error — it is
        // `FailedOperation.JobNotFound`, which is the same answer as a job that
        // never existed, on a call that has been billed.
        //
        // Driven through `poll_call`, which is the function `query` actually
        // uses. It takes the ticket and no `&self`, so a backend pointed
        // somewhere else — which is what this asserts — has no way to influence
        // the answer.
        let backend = backend();
        assert_eq!(backend.region(), Region::ApSingapore);

        let ticket = JobTicket::new("1338-abc", Endpoint::new(Region::EuFrankfurt));
        let body = wire::query_body(ticket.job_id());
        let call = poll_call(&ticket, &body);
        assert_eq!(call.region, "eu-frankfurt", "the ticket's region, not the backend's");
        assert_eq!(call.action, endpoint::QUERY);

        // And the region reaches the wire as a header, which is the only place it
        // appears: `wire::query_body` names only the job. It is sent *unsigned*,
        // as a common parameter, so a mismatch would still produce a perfectly
        // well-formed request — which is why this is checked here rather than
        // being left to a signature failure to catch.
        let signed = sign(
            &call,
            &Credentials::new("AKIDz", SecretKey::new("Gu5t9xGARNpq86cd98joQYCN3Cozk1qA")),
            1551113065,
        );
        let region: Vec<&str> = signed
            .headers()
            .filter(|(name, _)| *name == "X-TC-Region")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(region, ["eu-frankfurt"]);
        assert!(!body.contains("frankfurt"), "the body carries no region at all");
    }

    #[test]
    fn every_call_carries_the_content_type_that_was_signed() {
        // `sign.rs`'s warning, applied: an HTTP client that picks its own
        // `Content-Type` sends a request different from the one that was signed,
        // and the answer is an `AuthFailure.SignatureFailure` that reads as a bad
        // key. Which is why the body is a string handed to `body()` rather than a
        // value handed to `json()`.
        let signed = sign(
            &Endpoint::new(Region::EuFrankfurt).call(endpoint::SUBMIT, "{}"),
            &Credentials::new("AKIDz", SecretKey::new("k")),
            1551113065,
        );
        let sent: Vec<(&str, &str)> = signed.headers().collect();
        assert!(sent.contains(&("Content-Type", CONTENT_TYPE)));
        assert!(sent.iter().any(|(name, value)| *name == "X-TC-Version" && *value == "2023-09-01"));
        assert!(
            sent.iter()
                .any(|(name, value)| *name == "X-TC-Action" && *value == "SubmitHunyuanTo3DProJob")
        );
    }

    #[test]
    fn capabilities_differ_per_model_in_both_directions() {
        // 3.1 takes twice the views and loses two generate modes. One answer for
        // the backend would have to be the worse of the two, which throws away the
        // multi-view input that is the entire reason to prefer 3.1.
        let backend = backend();
        let pro = backend.capabilities(MODEL_3_1);
        assert_eq!(pro.max_views, 8);
        assert!(!pro.supports(GenerateType::Sketch), "unavailable on 3.1");
        assert!(!pro.supports(GenerateType::LowPoly));

        let older = backend.capabilities(MODEL_3_0);
        assert_eq!(older.max_views, 4);
        assert!(older.supports(GenerateType::Sketch), "and 3.0 still has it");
    }

    #[test]
    fn a_model_we_have_never_heard_of_gets_the_intersection_and_not_the_union() {
        // A project may name a model that has been retired, or one released next
        // month. Offering eight views to something that takes four is a paid call
        // refused after several megabytes have been uploaded, so the conservative
        // answer is the one both known models can honour.
        let caps = backend().capabilities("4.0-preview");
        assert_eq!(caps.max_views, 4, "the smaller of the two");
        assert!(caps.supports(GenerateType::Normal));
        assert!(!caps.supports(GenerateType::Sketch), "which 3.1 does not have");
        assert!(caps.requires_billing, "and this one is known without asking");
    }

    #[test]
    fn the_obj_result_is_unzipped_into_a_mesh_and_the_files_it_references() {
        // The trap `docs/08-providers.md` names: the OBJ `Url` is a `.zip` of the
        // mesh, its `.mtl` and its textures. Written straight to `model.obj` those
        // bytes are a file every viewer refuses, and nobody finds out until after
        // the 24-hour URL has expired.
        let (format, mesh, extras) = assemble(archive::tests::obj_archive()).unwrap();
        assert_eq!(format, MeshFormat::Obj);
        assert_eq!(mesh.name, "model.obj", "and it keeps its name, because the .mtl names it");
        let names: Vec<&str> = extras.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["model.mtl", "texture_0.png"]);
        assert!(!format.is_self_contained(), "so none of those may be dropped");
    }

    #[test]
    fn a_bare_glb_is_taken_as_it_is_rather_than_looked_for_inside_an_archive() {
        // The other documented shape, and the one the international docs
        // contradict. Deciding from the magic rather than from the declared
        // `Type` is what makes both work without trusting either.
        let bytes = [&archive::GLB_MAGIC[..], &2u32.to_le_bytes()[..], &[0u8; 8][..]].concat();
        let (format, mesh, extras) = assemble(bytes.clone()).unwrap();
        assert_eq!(format, MeshFormat::Glb);
        assert_eq!(mesh.bytes, bytes);
        assert!(extras.is_empty(), "a glb carries its own textures");
        assert!(format.is_self_contained());
    }

    #[test]
    fn the_mesh_inside_an_archive_is_picked_by_format_and_not_by_size() {
        // A texture map is routinely larger than the mesh that uses it, so
        // "the biggest file" is a heuristic that picks the albedo map. And an
        // archive with a `.glb` in it prefers that over the `.obj` beside it,
        // because a self-contained container cannot lose its materials.
        let bytes = archive::tests::zip(
            &[
                ("readme.txt", b"generated by hunyuan3d" as &[u8]),
                ("texture_0.png", &[0u8; 512]),
                ("model.obj", b"v 0 0 0\n"),
                ("model.glb", b"glTF small"),
            ],
            false,
        );
        let (format, mesh, extras) = assemble(bytes).unwrap();
        assert_eq!(format, MeshFormat::Glb);
        assert_eq!(mesh.name, "model.glb");
        assert_eq!(extras.len(), 3, "and nothing is thrown away");
    }

    #[test]
    fn a_download_that_is_neither_shape_says_what_arrived_instead() {
        // Almost always an XML error document from object storage — an expired
        // signature, a bucket policy — and its first line is the whole diagnosis.
        // Reported as "not a mesh" with nothing else, it is a dead end.
        let error =
            assemble(b"<?xml version=\"1.0\"?><Error><Code>AccessDenied".to_vec()).unwrap_err();
        assert!(error.to_string().contains("AccessDenied"), "{error}");
        assert!(matches!(error, Error::NotAMesh { .. }));

        // And an archive with nothing recognisable in it names what was in it,
        // rather than claiming the download failed.
        let junk = archive::tests::zip(&[("readme.txt", b"nothing here" as &[u8])], false);
        let error = assemble(junk).unwrap_err();
        assert!(error.to_string().contains("readme.txt"), "{error}");
    }

    #[test]
    fn a_preview_url_becomes_a_filename_without_its_signature() {
        // These are signed object-storage URLs and the signature is most of their
        // length. A filename built from the whole URL is unusable on every
        // filesystem there is.
        assert_eq!(
            preview_name("https://cos.example/jobs/a3f9.png?q-signature=deadbeef"),
            "a3f9.png"
        );
        assert_eq!(preview_name("https://cos.example/jobs/a3f9"), "preview.png");
        assert_eq!(preview_name(""), "preview.png");
    }

    #[test]
    fn a_backend_works_through_a_box_dyn_and_needs_no_network_to_build() {
        // `project.json` names the backend, so the generate path holds a
        // `Box<dyn MeshBackend>` — and the Inspector draws a backend dropdown on a
        // machine that has never had a Tencent key, so constructing one must not
        // touch the network.
        let boxed: Box<dyn MeshBackend> = Box::new(backend());
        assert_eq!(boxed.id(), "hunyuan3d");
        assert_eq!(boxed.label(), "Tencent Hunyuan3D");
        assert_eq!(boxed.default_model(), "3.1");
        assert!(boxed.capabilities(DEFAULT_MODEL).requires_billing);
    }

    #[test]
    fn debug_output_never_prints_either_half_of_an_account_wide_credential() {
        // A Tencent `SecretKey` is a master credential rather than a scoped token,
        // which `docs/08-providers.md` calls materially more dangerous to hold
        // than an OpenAI-style key. A derived `Debug` on the struct that owns one
        // is the realistic way it reaches a log file.
        let printed = format!("{:?}", backend());
        assert!(!printed.contains("Gu5t9x"), "{printed}");
        assert!(!printed.contains("AKIDzzzz"), "{printed}");
        assert!(printed.contains("ApSingapore"), "the region is not a secret: {printed}");
    }

    #[test]
    fn the_multi_view_path_is_the_one_the_default_model_takes() {
        // The lucky finding `docs/08-providers.md` describes: 3.1's headline
        // feature is multi-view input and the Turnaround preset is a multi-view
        // generator. If the default model ever stopped accepting eight views this
        // is the pairing that would break, silently, into a worse mesh.
        let views: Vec<MeshView> = View::ALL
            .into_iter()
            .map(|view| MeshView::new(view, vec![0x89, b'P', b'N', b'G'], "image/png"))
            .collect();
        let request = MeshRequest::from_views(DEFAULT_MODEL, views);
        let caps = backend().capabilities(&request.model);
        assert_eq!(caps.max_views, 8);
        assert!(wire::submit_body(&request, &caps).is_ok());
    }

    #[test]
    fn a_zero_length_wait_finishes_without_a_thread_and_a_real_one_wakes_up() {
        // The timer belongs to no runtime, which is what keeps `Cargo.toml`'s
        // claim that this crate names none true of the crate rather than only of
        // its documentation. Both halves are worth pinning: a zero that spawned a
        // thread would make the poll tests spawn hundreds, and a real sleep that
        // lost its wakeup is a generation that hangs until the process ends.
        block_on(sleep(Duration::ZERO));
        let started = Instant::now();
        block_on(sleep(Duration::from_millis(30)));
        assert!(started.elapsed() >= Duration::from_millis(25), "it did not actually wait");
    }
}
