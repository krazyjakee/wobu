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
//!    returns a `JobTicket` that owns its `Endpoint`, and the poll builds its
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
//! here: `JobTicket` is not serialisable, because a resumable-looking record with
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
mod tests;
