//! Everything Tencent says, and everything we say to it.
//!
//! The same split `comfy/wire.rs` and `gemini/wire.rs` make, and the one this
//! provider needs most: there are no credentials in the tree, so nothing here has
//! ever run against the real API. What can be checked is that the bodies we build
//! have the documented shape and that the answers in the documented shapes are
//! read correctly, and every function in this file is pure so that it can be.
//!
//! Two things about this API are unlike either of the other two:
//!
//! - **Failures arrive inside a 200.** Tencent puts `Response.Error.Code` in the
//!   body of an otherwise successful HTTP response, so there is no status code to
//!   switch on and the dotted code string is the whole of the signal. A reader
//!   that checked the status and then deserialised the success shape would see a
//!   job with no id and report "the backend returned nothing".
//! - **A job can fail twice over.** The submit can be refused, and the *query* can
//!   report `Status: FAIL` with its own `ErrorCode` — minutes later, after the job
//!   was billed. Both go through [`failure`] so the same code produces the same
//!   sentence whichever half of the run it arrived in.
//!
//! ## What is unverified
//!
//! Marked 🚩 at each site. The actions, the `Status` vocabulary, `ResultFile3Ds`,
//! `Model`, `EnablePBR`, `FaceCount` and `GenerateType` are all named directly in
//! `docs/08-providers.md`. The *sub-field* names inside `MultiViewImages`, and the
//! choice of `ImageBase64` over a single-entry `MultiViewImages` for a one-image
//! request, are read from Tencent's parameter documentation rather than from a
//! call that succeeded, and are the first things to check if a submit comes back
//! `InvalidParameterValue`.

use base64::Engine;
use serde_json::{Value, json};

use crate::error::Error;
use crate::mesh::{MeshCapabilities, MeshInput, MeshRequest, MeshView, View};

use super::sign::{BACKEND, auth_failure};

/// The largest total the provider takes, measured **before** base64 encoding.
///
/// `docs/08-providers.md` gives 6 MB for `ImageBase64` and 6 MB for a whole
/// multi-view set, and notes that base64 inflates by about 30%. Checked here
/// rather than left to the provider because the alternative is a request that
/// carries several megabytes across the world to be refused.
const MAX_IMAGE_BYTES: usize = 6 * 1024 * 1024;

/// What multi-view input accepts, and it is narrower than single-image input —
/// which takes webp as well.
const MULTI_VIEW_MIMES: [&str; 2] = ["image/jpeg", "image/png"];

/* ── what we send ─────────────────────────────────────────────────────────── */

/// The body of `SubmitHunyuanTo3DProJob`.
///
/// A `String` rather than a `Value`, because the signature is computed over the
/// exact bytes that go on the socket: a body re-serialised anywhere between here
/// and the send is an `AuthFailure.SignatureFailure`, which reads as a bad key
/// and sends the user to regenerate an account-wide master credential.
///
/// Every check in it refuses a request the provider would refuse, before it is
/// signed and sent. That is the trait's first contract point — send exactly what
/// the request says or fail — and it is [`Error::Unsupported`] rather than a
/// clamp, because a mesh silently generated at a different face count is a paid
/// job the user did not ask for.
pub(crate) fn submit_body(
    request: &MeshRequest,
    capabilities: &MeshCapabilities,
) -> Result<String, Error> {
    if !capabilities.face_count.contains(&request.face_count) {
        return Err(Error::Unsupported {
            detail: format!(
                "{BACKEND} takes between {} and {} faces and this request asks for {}",
                capabilities.face_count.start(),
                capabilities.face_count.end(),
                request.face_count,
            ),
        });
    }
    if !capabilities.supports(request.generate_type) {
        return Err(Error::Unsupported {
            detail: format!(
                "the {} generate mode is not available on model {} — it offers {}",
                request.generate_type,
                request.model,
                capabilities
                    .generate_types
                    .iter()
                    .map(|mode| mode.as_str())
                    .collect::<Vec<_>>()
                    .join(" and "),
            ),
        });
    }

    let mut body = json!({
        // A parameter and not an endpoint. `docs/08-providers.md`: the default is
        // "3.0" and 3.1 is Pro-only, so this is always sent rather than omitted —
        // an omitted `Model` is a silently older reconstruction at the same price.
        "Model": request.model,
        "GenerateType": request.generate_type.as_str(),
        "EnablePBR": request.enable_pbr,
        "FaceCount": request.face_count,
    });

    match &request.input {
        MeshInput::Prompt(prompt) => {
            if !capabilities.text_to_mesh {
                return Err(Error::Unsupported {
                    detail: format!("model {} does not reconstruct from text alone", request.model),
                });
            }
            if prompt.trim().is_empty() {
                return Err(Error::Unsupported {
                    detail: "a text-to-mesh request with an empty prompt".to_owned(),
                });
            }
            body["Prompt"] = json!(prompt);
        }
        MeshInput::Views(views) => {
            check_views(views, capabilities)?;
            match views.as_slice() {
                // 🚩 Single-image input is documented as `ImageBase64`/`ImageUrl`
                // rather than a one-entry `MultiViewImages`, and unverified live.
                // If a one-image submit comes back `InvalidParameterValue`, moving
                // it into the multi-view array is the first thing to try.
                [only] => body["ImageBase64"] = json!(encode(&only.bytes)),
                many => {
                    body["MultiViewImages"] = Value::Array(
                        many.iter()
                            .map(|view| {
                                // 🚩 `ViewType` and `ViewImageBase64` are from the
                                // parameter documentation, not from a call that
                                // succeeded.
                                json!({
                                    "ViewType": view.view.as_str(),
                                    "ViewImageBase64": encode(&view.bytes),
                                })
                            })
                            .collect(),
                    )
                }
            }
        }
    }

    Ok(body.to_string())
}

/// The body of `QueryHunyuanTo3DProJob`.
///
/// The region does not appear in it — it is the `X-TC-Region` header, which
/// `Endpoint::call` fills from the ticket. That is why the poll cannot go to the
/// wrong place: there is nothing about a region in this function to get wrong.
pub(crate) fn query_body(job_id: &str) -> String {
    json!({ "JobId": job_id }).to_string()
}

/// Every reason a set of views would be refused, checked before anything is sent.
fn check_views(views: &[MeshView], capabilities: &MeshCapabilities) -> Result<(), Error> {
    if views.is_empty() {
        return Err(Error::Unsupported {
            detail: "an image-to-mesh request with no images in it".to_owned(),
        });
    }
    if views.len() > capabilities.max_views {
        return Err(Error::Unsupported {
            detail: format!(
                "{BACKEND} takes at most {} views on this model and this request has {}",
                capabilities.max_views,
                views.len(),
            ),
        });
    }

    // "Exactly one image per view type, no duplicates" (`docs/08-providers.md`).
    // A duplicate is not a warning at the provider, it is a rejection — and the
    // realistic way to produce one is a Turnaround batch where two images were
    // tagged with the same view.
    let mut seen: Vec<View> = Vec::with_capacity(views.len());
    for view in views {
        if seen.contains(&view.view) {
            return Err(Error::Unsupported {
                detail: format!("two images are tagged as the `{}` view", view.view),
            });
        }
        seen.push(view.view);
    }

    // Multi-view is narrower than single-image, which also takes webp. Sending a
    // webp in a multi-view set is a rejection after the upload.
    if views.len() > 1 {
        for view in views {
            if !MULTI_VIEW_MIMES.contains(&view.mime.to_ascii_lowercase().as_str()) {
                return Err(Error::Unsupported {
                    detail: format!(
                        "the `{}` view is {} and multi-view input takes only JPEG and PNG",
                        view.view, view.mime,
                    ),
                });
            }
        }
    }

    let total: usize = views.iter().map(|view| view.bytes.len()).sum();
    if total > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported {
            detail: format!(
                "these {} views are {:.1} MB together and {BACKEND} takes {} MB",
                views.len(),
                total as f64 / (1024.0 * 1024.0),
                MAX_IMAGE_BYTES / (1024 * 1024),
            ),
        });
    }
    Ok(())
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/* ── what comes back ──────────────────────────────────────────────────────── */

/// Where a job has got to.
///
/// `WAIT | RUN | FAIL | DONE` is the documented vocabulary, exactly.
/// [`Status::Unknown`] is not a fifth state the provider has — it is what a
/// renamed or added one decodes to, and it is treated as **not terminal** so that
/// a rename of `RUN` is a poll that keeps going rather than a job abandoned while
/// it generates. The poll deadline is what stops that being an infinite loop, and
/// the timeout message names the status so the next person knows what to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Wait,
    Run,
    Done,
    Fail,
    Unknown(String),
}

impl Status {
    fn parse(raw: &str) -> Status {
        match raw {
            "WAIT" => Status::Wait,
            "RUN" => Status::Run,
            "DONE" => Status::Done,
            "FAIL" => Status::Fail,
            other => Status::Unknown(other.to_owned()),
        }
    }

    /// What the status bar says while this is happening.
    pub(crate) fn note(&self) -> String {
        match self {
            Status::Wait => format!("queued at {BACKEND}"),
            Status::Run => "generating the mesh".to_owned(),
            Status::Done => "downloading the mesh".to_owned(),
            Status::Fail => "failed".to_owned(),
            Status::Unknown(raw) => format!("status {raw}"),
        }
    }
}

/// One entry of `ResultFile3Ds`.
///
/// `kind` is the provider's `Type` and is kept as a **string**.
/// `docs/08-providers.md` asks for exactly that: "the international docs list
/// `Type` values that contradict GLB being returned — treat `Type` as an open
/// string and switch on it defensively rather than as an enum". It is used only
/// to *order* the candidates; what a download turns out to be is decided from its
/// bytes, in `archive.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResultFile {
    pub(crate) kind: String,
    pub(crate) url: String,
    /// The provider's own render. Expires on the same 24-hour clock as `url`,
    /// which is why it is downloaded rather than remembered.
    pub(crate) preview: Option<String>,
}

/// What one poll found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Progress {
    pub(crate) status: Status,
    pub(crate) files: Vec<ResultFile>,
    /// Set when `Status` is `FAIL`. Carried separately from the top-level
    /// `Error.Code` because it arrives on a **successful** query about a job that
    /// failed, minutes after it was billed.
    pub(crate) failure: Option<(String, String)>,
}

/// The `JobId` a submit came back with, or the failure it came back with instead.
pub(crate) fn submitted(body: &[u8]) -> Result<String, Error> {
    let response = response(body)?;
    match response.get("JobId").and_then(Value::as_str) {
        Some(job_id) if !job_id.is_empty() => Ok(job_id.to_owned()),
        // A 200 with neither an error nor an id. Reported as unavailable rather
        // than as a bad response because there is nothing to retry *differently*
        // and waiting is the sensible thing to do.
        _ => Err(Error::Unavailable {
            detail: format!("{BACKEND} accepted the job and gave it no id"),
        }),
    }
}

/// One poll, decoded.
pub(crate) fn progress(body: &[u8]) -> Result<Progress, Error> {
    let response = response(body)?;
    let text = |key: &str| response.get(key).and_then(Value::as_str).unwrap_or_default();
    let status = Status::parse(text("Status"));

    let failure = match &status {
        Status::Fail => Some((
            match text("ErrorCode") {
                // A FAIL with no code at all still has to say something, or the
                // job ends with an empty sentence.
                "" => "FailedOperation".to_owned(),
                code => code.to_owned(),
            },
            text("ErrorMessage").to_owned(),
        )),
        _ => None,
    };

    let files = response
        .get("ResultFile3Ds")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let url = entry.get("Url").and_then(Value::as_str)?;
                    if url.is_empty() {
                        return None;
                    }
                    Some(ResultFile {
                        kind: entry
                            .get("Type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        url: url.to_owned(),
                        preview: entry
                            .get("PreviewImageUrl")
                            .and_then(Value::as_str)
                            .filter(|preview| !preview.is_empty())
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Progress { status, files, failure })
}

/// The result files to try, best first.
///
/// **Ordered by the `Type` hint and never filtered by it.** A self-contained
/// container is preferred because it cannot lose its textures on the way to disk;
/// an archive is next because that is what the OBJ entry actually is; and
/// anything else is still tried, because the docs contradict themselves about
/// which types exist and a job that has been paid for should not be thrown away
/// on the strength of an unrecognised four-letter string.
///
/// Matching is on a lowercased *substring*, so `GLB`, `glb`, `model/gltf-binary`
/// and `3D_GLB` all sort the same way. Anything more precise would be a list of
/// spellings, which is the thing being defended against.
pub(crate) fn candidates(files: &[ResultFile]) -> Vec<&ResultFile> {
    let rank = |file: &ResultFile| {
        let kind = file.kind.to_ascii_lowercase();
        if kind.contains("glb") || kind.contains("gltf") {
            0
        } else if kind.contains("obj") || kind.contains("zip") {
            1
        } else {
            2
        }
    };
    let mut ordered: Vec<&ResultFile> = files.iter().collect();
    ordered.sort_by_key(|file| rank(file));
    ordered
}

/// The `Response` object, or the error inside it.
///
/// Every Tencent answer is wrapped in one, success and failure alike, and the
/// failure arrives with HTTP 200 — so this is the only place either shape is
/// recognised.
fn response(body: &[u8]) -> Result<Value, Error> {
    let document: Value = serde_json::from_slice(body).map_err(|_| Error::Unavailable {
        detail: format!(
            "{BACKEND} answered with {} bytes that are not JSON, which is usually a proxy or a \
             captive portal rather than the API",
            body.len(),
        ),
    })?;
    let Some(response) = document.get("Response") else {
        return Err(Error::Unavailable {
            detail: format!("{BACKEND} answered with JSON that has no Response in it"),
        });
    };
    if let Some(error) = response.get("Error") {
        let text = |key: &str| error.get(key).and_then(Value::as_str).unwrap_or_default();
        return Err(failure(text("Code"), text("Message")));
    }
    Ok(response.clone())
}

/// A Tencent error code, turned into something a person can act on.
///
/// The codes below are the ones with a *different fix* behind them. Everything
/// else falls through to [`Error::Unavailable`] carrying the code, which is the
/// only description of the problem that exists — and retryable, which is safe
/// here in a way it would not be elsewhere: a submit that failed has no `JobId`
/// and so was not billed, and a query that reports `FAIL` returns a
/// [`MeshUsage`](crate::MeshUsage) of one, which is what makes `wobu-jobs` hold
/// the retry for the person paying rather than spending it.
pub(crate) fn failure(code: &str, message: &str) -> Error {
    // The `AuthFailure` family is `sign.rs`'s, including the `SignatureExpire`
    // split that sends the user to their system clock rather than to Settings.
    // Asked first so that no code below can shadow it.
    if let Some(error) = auth_failure(code) {
        return error;
    }

    let said = || match message.trim() {
        "" => format!("{BACKEND} said {code}"),
        message => format!("{message} ({code})"),
    };

    match code {
        // `docs/08-providers.md` wants this to be an onboarding step with a link
        // rather than a runtime error, and until it is, this sentence is the
        // whole of what the user gets. A fresh account hits it before anything
        // works, and nothing about the key or the request is wrong.
        "FailedOperation.ServiceNotActivated" | "ResourceUnavailable.NotExist" => {
            Error::Unavailable {
                detail: format!(
                    "{BACKEND} is not switched on for this Tencent Cloud account. Activate the 3D \
                     service in the Tencent Cloud console, then try again — nothing else about \
                     your key is wrong"
                ),
            }
        }

        // The wrong namespace, which is the single most likely way to get this
        // adapter wrong and which `endpoint.rs` exists to prevent. If it ever
        // arrives, it is ours and not the user's.
        "ResourceUnavailable.InterfaceNotExist" | "InvalidAction" | "InvalidVersion" => {
            Error::Unsupported {
                detail: format!(
                    "{BACKEND} has no such action at this host and version — wobu is calling the \
                     wrong namespace ({code}). See the table in docs/08-providers.md"
                ),
            }
        }

        // Only three regions work and only three are offered, so this is a bug in
        // `endpoint.rs` rather than something the user chose.
        "UnsupportedRegion" | "InvalidParameterValue.Region" => Error::Unsupported {
            detail: format!(
                "{BACKEND} does not serve this region — wobu should only ever send one of the \
                 three it supports ({code})"
            ),
        },

        // The 24-hour `JobId` expiry, and the other thing that produces it: a
        // poll aimed at a different region from the submit. Both look identical
        // from here, so the sentence names both.
        "FailedOperation.JobNotFound" | "ResourceNotFound" | "ResourceNotFound.JobNotExist" => {
            Error::Unavailable {
                detail: format!(
                    "{BACKEND} has no record of this job. A job id is only valid for 24 hours, \
                     and it can only be asked about in the region it was submitted to"
                ),
            }
        }

        // Money. Its own variant because it sends the user to a billing console
        // and nowhere near their key, which is the failure `error.rs` says
        // arrives as a neighbouring code and is worth a whole sentence.
        "FailedOperation.BalanceInsufficient"
        | "FailedOperation.ArrearsError"
        | "FailedOperation.ChargeStatusException"
        | "FailedOperation.ResourcePackExhausted"
        | "FailedOperation.NoFreeAmount" => Error::BillingRequired {
            backend: BACKEND,
            detail: format!(
                "top up or enable pay-as-you-go for the Hunyuan 3D service in the Tencent Cloud \
                 console ({code})"
            ),
        },

        // The rate limit and the concurrency cap. `retry_after` is `None` because
        // Tencent sends no hint with either, and the queue's own backoff is a
        // better guess than one invented here.
        "RequestLimitExceeded"
        | "RequestLimitExceeded.JobNumExceed"
        | "RequestLimitExceeded.UinLimitExceeded"
        | "LimitExceeded" => Error::RateLimited { backend: BACKEND, retry_after: None },

        _ => classify(code, said()),
    }
}

/// The codes that come in families rather than as single strings.
///
/// Prefix matching, kept apart from the exact table above so that the exact
/// answers always win. Content moderation in particular is spelled several ways
/// across Tencent's products and the family is more stable than any one member.
fn classify(code: &str, said: String) -> Error {
    let lower = code.to_ascii_lowercase();

    // A moderation refusal. `error.rs` makes this retryable on purpose: the same
    // input does sometimes pass on a later attempt, and whether that is worth
    // paying for is the usage figure's question rather than this one's.
    if lower.contains("illegal") || lower.contains("moderation") || lower.contains("sensitive") {
        return Error::Refused { detail: said };
    }

    // We built the request, so a rejected parameter is our bug and must not
    // offer the user a "Try again" that will be refused identically.
    if lower.starts_with("invalidparameter") || lower.starts_with("missingparameter") {
        return Error::Unsupported { detail: said };
    }

    // The images we sent. Also ours — `wire.rs` checks the format and the size
    // budget before signing, so reaching this means our checks and their decoder
    // disagree, and the detail is what says which.
    if lower.contains("imagedecode")
        || lower.contains("imagedownload")
        || lower.contains("imagesize")
    {
        return Error::Unsupported { detail: said };
    }

    Error::Unavailable { detail: said }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{FACE_COUNT, GenerateType, MeshView};

    fn caps() -> MeshCapabilities {
        MeshCapabilities {
            max_views: 8,
            face_count: FACE_COUNT,
            pbr: true,
            generate_types: vec![GenerateType::Normal, GenerateType::Geometry],
            text_to_mesh: true,
            requires_billing: true,
        }
    }

    fn view(view: View) -> MeshView {
        MeshView::new(view, vec![0x89, b'P', b'N', b'G'], "image/png")
    }

    fn body(request: &MeshRequest) -> Value {
        serde_json::from_str(&submit_body(request, &caps()).unwrap()).unwrap()
    }

    #[test]
    fn the_submit_body_carries_the_model_as_a_parameter_because_3_1_is_not_an_endpoint() {
        // `docs/08-providers.md`: "3.1 is a parameter, not an endpoint", the
        // default is 3.0, and it is Pro-only. An omitted `Model` is a silently
        // older reconstruction at the same price, which is invisible in the
        // response and visible only in the mesh.
        let sent = body(&MeshRequest::from_prompt("3.1", "a wrought iron lantern"));
        assert_eq!(sent["Model"], "3.1");
        assert_eq!(sent["Prompt"], "a wrought iron lantern");
        assert_eq!(sent["GenerateType"], "Normal");
        assert_eq!(sent["EnablePBR"], false);
        assert_eq!(sent["FaceCount"], 500_000);
    }

    #[test]
    fn eight_views_go_in_the_multi_view_array_and_one_goes_in_the_single_image_field() {
        // 3.1's headline feature, and the reason the Turnaround preset emits
        // exactly these eight names. The single-image case is a different
        // parameter rather than a one-entry array.
        let all = MeshRequest::from_views("3.1", View::ALL.into_iter().map(view).collect());
        let sent = body(&all);
        let names: Vec<&str> = sent["MultiViewImages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["ViewType"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["front", "left", "right", "back", "top", "bottom", "left_front", "right_front"]
        );
        assert!(
            sent["MultiViewImages"][0]["ViewImageBase64"].as_str().unwrap().starts_with("iVBORw")
        );
        assert!(sent.get("ImageBase64").is_none());

        let one = MeshRequest::from_views("3.1", vec![view(View::Front)]);
        let sent = body(&one);
        assert!(sent["ImageBase64"].as_str().unwrap().starts_with("iVBORw"));
        assert!(sent.get("MultiViewImages").is_none());
    }

    #[test]
    fn two_images_tagged_as_the_same_view_are_refused_before_anything_is_sent() {
        // "Exactly one image per view type, no duplicates". The realistic way to
        // produce one is a Turnaround batch where two images got the same tag, and
        // the provider's answer is a rejection after several megabytes have
        // crossed the world.
        let request = MeshRequest::from_views(
            "3.1",
            vec![view(View::Front), view(View::Left), view(View::Front)],
        );
        let error = submit_body(&request, &caps()).unwrap_err();
        assert!(error.to_string().contains("`front`"), "{error}");
        assert_eq!(error.code(), "internal", "we built it, so it is our bug");
    }

    #[test]
    fn a_face_count_outside_the_range_is_refused_rather_than_clamped_to_it() {
        // The trait's first contract point. A mesh quietly generated at 3000
        // faces when 1000 was asked for is a paid job for something nobody
        // wanted, and nothing in the response says the number changed.
        for faces in [2_999, 1_500_001] {
            let request = MeshRequest::from_prompt("3.1", "p").with_face_count(faces);
            let error = submit_body(&request, &caps()).unwrap_err();
            assert!(error.to_string().contains("3000 and 1500000"), "{error}");
            assert!(error.to_string().contains(&faces.to_string()), "{error}");
            assert_eq!(error.code(), "internal", "we built it, so it is our bug");
        }
        for faces in [3_000, 500_000, 1_500_000] {
            let request = MeshRequest::from_prompt("3.1", "p").with_face_count(faces);
            assert_eq!(body(&request)["FaceCount"], faces);
        }
    }

    #[test]
    fn the_two_generate_modes_3_1_lost_are_refused_by_name() {
        // `docs/08-providers.md`: `LowPoly` and `Sketch` are unavailable on 3.1.
        // Sending one is a paid call refused as an invalid parameter, and the
        // message the provider gives does not say which model dropped it.
        for mode in [GenerateType::LowPoly, GenerateType::Sketch] {
            let request = MeshRequest::from_prompt("3.1", "p").with_generate_type(mode);
            let error = submit_body(&request, &caps()).unwrap_err();
            assert!(error.to_string().contains(mode.as_str()), "{error}");
            assert!(error.to_string().contains("Normal and Geometry"), "{error}");
        }
        assert_eq!(
            body(&MeshRequest::from_prompt("3.1", "p").with_generate_type(GenerateType::Geometry))
                ["GenerateType"],
            "Geometry",
        );
    }

    #[test]
    fn a_multi_view_set_in_a_format_only_single_image_input_takes_is_refused() {
        // Single-image input takes webp and multi-view does not. The narrower of
        // the two limits is the one that applies, and the provider's rejection
        // arrives after the upload.
        let request = MeshRequest::from_views(
            "3.1",
            vec![
                view(View::Front),
                MeshView::new(View::Left, vec![b'R', b'I', b'F', b'F'], "image/webp"),
            ],
        );
        let error = submit_body(&request, &caps()).unwrap_err();
        assert!(error.to_string().contains("image/webp"), "{error}");
        assert!(error.to_string().contains("JPEG and PNG"), "{error}");
    }

    #[test]
    fn a_view_set_over_the_size_budget_never_leaves_the_machine() {
        // Six megabytes measured before base64, which inflates by about 30%.
        // Checked here because the alternative is carrying seven megabytes across
        // the world to be told no.
        let heavy = |v| MeshView::new(v, vec![0u8; 4 * 1024 * 1024], "image/png");
        let request = MeshRequest::from_views("3.1", vec![heavy(View::Front), heavy(View::Left)]);
        let error = submit_body(&request, &caps()).unwrap_err();
        assert!(error.to_string().contains("8.0 MB"), "{error}");
        assert!(error.to_string().contains("6 MB"), "{error}");
    }

    #[test]
    fn more_views_than_the_model_takes_is_refused_with_the_number_it_takes() {
        // 3.0 accepts front + 3 and 3.1 accepts front + 7, so this is per model
        // rather than per backend. Sending eight to a 3.0 job is a rejection that
        // does not say which four were too many.
        let three_oh = MeshCapabilities { max_views: 4, ..caps() };
        let request = MeshRequest::from_views("3.0", View::ALL.into_iter().map(view).collect());
        let error = submit_body(&request, &three_oh).unwrap_err();
        assert!(error.to_string().contains("at most 4"), "{error}");
        assert!(error.to_string().contains("has 8"), "{error}");
    }

    #[test]
    fn the_query_body_names_only_the_job_because_the_region_is_a_header() {
        // The whole of why a poll cannot go to the wrong region: there is nothing
        // about one in this function to get wrong. `Endpoint::call` fills
        // `X-TC-Region` from the ticket that the submit produced.
        assert_eq!(query_body("1338-abc"), r#"{"JobId":"1338-abc"}"#);
    }

    /* ── the answers ──────────────────────────────────────────────────── */

    #[test]
    fn a_submit_returns_a_job_id_out_of_the_response_wrapper() {
        // Every Tencent answer is wrapped in `Response`, success and failure
        // alike. A reader that deserialised the top level would find no `JobId`
        // and report a backend that gave the job no id.
        let ok = br#"{"Response":{"JobId":"1338-abc","RequestId":"7f6a"}}"#;
        assert_eq!(submitted(ok).unwrap(), "1338-abc");

        let empty = br#"{"Response":{"RequestId":"7f6a"}}"#;
        assert!(submitted(empty).unwrap_err().to_string().contains("gave it no id"));
    }

    #[test]
    fn a_failure_arrives_inside_a_200_and_is_read_out_of_the_body() {
        // The thing that is unlike both other providers. There is no status code
        // to switch on; the dotted string in the body is the whole of the signal,
        // and a reader that trusted the 200 would report an empty success.
        let body = br#"{"Response":{"Error":{"Code":"FailedOperation.ServiceNotActivated",
            "Message":"service not activated"},"RequestId":"7f6a"}}"#;
        let error = submitted(body).unwrap_err();
        assert!(error.to_string().contains("not switched on"), "{error}");
        assert!(error.to_string().contains("console"), "{error}");
        assert!(error.is_retryable(), "activating it and pressing Try again is the fix");
    }

    #[test]
    fn the_documented_status_vocabulary_decodes_and_anything_else_keeps_polling() {
        // `WAIT | RUN | FAIL | DONE`, exactly. A fifth string is a rename or an
        // addition, and treating it as a failure would abandon a job mid-generate
        // because the provider shipped a release; treating it as terminal success
        // would return no mesh. It is neither — the poll deadline is what bounds
        // it, and the note names it.
        let wire = |status: &str| {
            let body = format!(r#"{{"Response":{{"Status":"{status}","RequestId":"7f"}}}}"#);
            progress(body.as_bytes()).unwrap().status
        };
        assert_eq!(wire("WAIT"), Status::Wait);
        assert_eq!(wire("RUN"), Status::Run);
        assert_eq!(wire("DONE"), Status::Done);
        assert_eq!(wire("FAIL"), Status::Fail);
        assert_eq!(wire("QUEUED"), Status::Unknown("QUEUED".into()));
        assert_eq!(wire("QUEUED").note(), "status QUEUED");
    }

    #[test]
    fn a_done_job_yields_its_files_with_the_type_left_as_the_string_it_arrived_as() {
        // `docs/08-providers.md` asks for `Type` to be treated as an open string
        // because the international docs contradict GLB being returned. Decoding
        // it into an enum here would make an unrecognised type a failed job that
        // has already been paid for.
        let body = br#"{"Response":{"Status":"DONE","ErrorCode":"","ErrorMessage":"",
            "ResultFile3Ds":[{"Type":"OBJ","Url":"https://cos.example/a.zip",
            "PreviewImageUrl":"https://cos.example/a.png"}],"RequestId":"7f"}}"#;
        let found = progress(body).unwrap();
        assert_eq!(found.status, Status::Done);
        assert_eq!(found.files[0].kind, "OBJ");
        assert_eq!(found.files[0].url, "https://cos.example/a.zip");
        assert_eq!(found.files[0].preview.as_deref(), Some("https://cos.example/a.png"));
        assert_eq!(found.failure, None);
    }

    #[test]
    fn an_unrecognised_result_type_is_tried_last_rather_than_thrown_away() {
        // The defensive switch, as ordering. A self-contained container first
        // because it cannot lose its textures; the archive next because that is
        // what the OBJ entry actually is; and the unknown one is still a
        // candidate, because the job has been paid for.
        let files = vec![
            ResultFile { kind: "SOMETHING_NEW".into(), url: "c".into(), preview: None },
            ResultFile { kind: "OBJ".into(), url: "b".into(), preview: None },
            ResultFile { kind: "model/gltf-binary".into(), url: "a".into(), preview: None },
        ];
        let urls: Vec<&str> = candidates(&files).iter().map(|file| file.url.as_str()).collect();
        assert_eq!(urls, ["a", "b", "c"]);
        assert_eq!(candidates(&files).len(), 3, "nothing is filtered out by its Type");

        // And an entry with no URL at all is not a candidate, because there is
        // nothing to fetch.
        let body = br#"{"Response":{"Status":"DONE","ResultFile3Ds":[{"Type":"GLB","Url":""}]}}"#;
        assert!(progress(body).unwrap().files.is_empty());
    }

    #[test]
    fn a_job_that_failed_after_it_was_billed_carries_its_own_code_and_not_the_wrappers() {
        // A `FAIL` arrives on a *successful* query, minutes after the submit was
        // charged. Read only from `Response.Error`, this would decode as a job
        // still in progress and the poll would run until the deadline.
        let body = br#"{"Response":{"Status":"FAIL","ErrorCode":"FailedOperation.ImageIllegal",
            "ErrorMessage":"image blocked by moderation","RequestId":"7f"}}"#;
        let found = progress(body).unwrap();
        assert_eq!(found.status, Status::Fail);
        let (code, message) = found.failure.unwrap();
        assert_eq!(code, "FailedOperation.ImageIllegal");
        assert!(matches!(failure(&code, &message), Error::Refused { .. }));
    }

    #[test]
    fn the_signature_expiry_still_reaches_the_clock_message_through_this_table() {
        // `sign.rs` owns the `AuthFailure` family and is asked first, so no code
        // added below can shadow the one mapping in the whole provider that sends
        // the user to their operating system's clock rather than to Settings.
        let skew = failure("AuthFailure.SignatureExpire", "expired");
        assert_eq!(skew.code(), "provider.clock_skew");
        assert!(skew.to_string().contains("system clock"), "{skew}");
        assert_eq!(failure("AuthFailure.SecretIdNotFound", "").code(), "provider.bad_key");
    }

    #[test]
    fn the_wrong_namespace_and_the_wrong_region_are_reported_as_our_bug() {
        // `endpoint.rs` exists so neither can happen. If either arrives anyway it
        // is a constant of ours, and offering the user a "Try again" for a call
        // that will be refused identically is worse than saying so.
        for code in ["ResourceUnavailable.InterfaceNotExist", "InvalidAction"] {
            let error = failure(code, "");
            assert_eq!(error.code(), "internal", "{code}");
            assert!(error.to_string().contains("namespace"), "{error}");
        }
        let region = failure("UnsupportedRegion", "");
        assert_eq!(region.code(), "internal");
        assert!(region.to_string().contains("three"), "{region}");
    }

    #[test]
    fn a_job_id_that_has_expired_says_so_and_names_the_other_thing_that_produces_it() {
        // `FailedOperation.JobNotFound` is what a 24-hour-old job id gives *and*
        // what a poll aimed at the wrong region gives. They are indistinguishable
        // from here, so the sentence has to name both or it sends half the people
        // who hit it to the wrong place.
        let error = failure("FailedOperation.JobNotFound", "job not found");
        assert!(error.to_string().contains("24 hours"), "{error}");
        assert!(error.to_string().contains("region it was submitted to"), "{error}");
    }

    #[test]
    fn arrears_send_the_user_to_a_billing_console_and_nowhere_near_their_key() {
        // The failure `error.rs` says arrives as a neighbouring code and sends
        // people to two different websites. A key that worked yesterday and an
        // account that ran out of credit look identical in the response.
        let error = failure("FailedOperation.BalanceInsufficient", "");
        assert_eq!(error.code(), "provider.billing_required");
        assert!(error.to_string().contains("Tencent Cloud console"), "{error}");
        assert!(!error.is_retryable(), "topping up is not something a retry does");
    }

    #[test]
    fn the_concurrency_and_rate_limits_are_reported_as_a_rate_limit_and_not_as_a_failure() {
        // Three concurrent Pro jobs and 20 requests a second. The queue's own cap
        // is what should keep us under both; if one arrives anyway it is worth
        // waiting on rather than worth failing, and no `retry_after` is invented
        // because Tencent sends none.
        for code in ["RequestLimitExceeded", "RequestLimitExceeded.JobNumExceed"] {
            match failure(code, "") {
                Error::RateLimited { retry_after, .. } => assert_eq!(retry_after, None, "{code}"),
                other => panic!("{code} became {other}"),
            }
        }
    }

    #[test]
    fn a_code_we_have_never_seen_carries_tencents_own_sentence() {
        // The fallback has to be better than "something went wrong", because the
        // code and the message are the only description of the problem in
        // existence — and this provider has a long tail of them.
        let error = failure("InternalError.SomethingNew", "the model is unavailable");
        assert!(error.to_string().contains("the model is unavailable"), "{error}");
        assert!(error.to_string().contains("InternalError.SomethingNew"), "{error}");

        // Even with no message at all, which some codes arrive with.
        assert!(failure("InternalError.Quiet", "").to_string().contains("InternalError.Quiet"));
    }

    #[test]
    fn a_rejected_parameter_is_our_bug_and_a_moderation_refusal_is_not() {
        // We build the request, so an invalid parameter must not offer a "Try
        // again" that will be refused identically. A moderation refusal is
        // retryable because the same input does sometimes pass — whether it is
        // worth paying for is the usage figure's question.
        assert_eq!(failure("InvalidParameterValue.FaceCount", "").code(), "internal");
        assert_eq!(failure("MissingParameter", "").code(), "internal");
        assert!(failure("FailedOperation.PromptIllegal", "").is_retryable());
        assert_eq!(failure("FailedOperation.TextSensitive", "").code(), "provider.bad_response");
    }

    #[test]
    fn a_body_that_is_not_json_reports_the_proxy_rather_than_panicking_on_it() {
        // A captive portal or a corporate proxy answers 200 with HTML. Reported
        // as a decode failure this reads as the provider being broken, which
        // sends the user to a status page instead of to their network.
        let error = submitted(b"<html>Sign in to continue</html>").unwrap_err();
        assert!(error.to_string().contains("not JSON"), "{error}");
        assert!(error.to_string().contains("proxy"), "{error}");
        assert!(progress(b"{}").unwrap_err().to_string().contains("no Response"));
    }
}
