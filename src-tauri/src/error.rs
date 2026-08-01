//! How errors travel from Rust to the UI.
//!
//! Each crate keeps its own `thiserror` enum — that is the right shape for
//! Rust, where an `io::Error` source and a `PathBuf` are useful. None of it is
//! the right shape for a webview, and some of it must not reach one at all.
//! So every crate's error is flattened here, at the command boundary, into a
//! single [`WobuError`], and there is exactly one constructor so that
//! [`redact::scrub`] cannot be bypassed.
//!
//! ## The four fields
//!
//! - `code` — a stable dotted string the frontend switches on. Stable means
//!   stable: these appear in `src/lib/api.ts` and in conditionals, so a code is
//!   renamed only alongside its call sites.
//! - `message` — one sentence for a person. Says what happened and, where
//!   there is one, what to do.
//! - `detail` — the technical remainder, for the diagnostics log (#7) and for
//!   a user copying something into an issue. Never required to understand the
//!   error, because the UI does not always show it.
//! - `retryable` — whether trying the same thing again could plausibly work.
//!   A share that is unmounted may come back; a parent cycle will not.
//!
//! ## Why the frontend decides toast vs banner
//!
//! It does not live here. `src/lib/api.ts` maps codes onto surfaces, because
//! whether something deserves a persistent banner is a question about the UI's
//! attention budget, not about what went wrong. What this file guarantees is
//! that the code is stable enough to map.

use serde::Serialize;
use wobu_store::Error as StoreError;

use crate::diag;
use crate::redact;

/// The stable codes. Serialised as the dotted string, not as the variant name.
///
/// Grouped by the part of the system that owns the failure, because that is
/// the axis the UI branches on: everything under `share.` is a banner,
/// everything under `provider.` sends the user to Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Code {
    // ── project ──────────────────────────────────────────────────────────
    /// The chosen folder has no `project.json`.
    #[serde(rename = "project.not_a_project")]
    NotAProject,
    /// Creating a project where one already exists.
    #[serde(rename = "project.already_exists")]
    AlreadyExists,
    /// Written by a newer Wobu than this build understands. Opening read-write
    /// would silently drop the fields this build cannot represent.
    #[serde(rename = "project.schema_too_new")]
    SchemaTooNew,
    /// A command needing an open project was called without one.
    #[serde(rename = "project.none_open")]
    NoProjectOpen,

    // ── node ─────────────────────────────────────────────────────────────
    /// No node with that id — usually a tab pointing at something deleted.
    #[serde(rename = "node.not_found")]
    NoSuchNode,
    /// A file on disk could not be parsed. Never a reason to overwrite it.
    #[serde(rename = "node.malformed")]
    Malformed,
    /// A domain rule said no: empty name, parent cycle, duplicate singleton.
    #[serde(rename = "node.invalid")]
    Invalid,

    // ── write ────────────────────────────────────────────────────────────
    /// A concurrent writer won the race; ours was parked alongside.
    #[serde(rename = "write.conflict")]
    Conflict,
    /// The project folder is not writable.
    #[serde(rename = "write.read_only")]
    ReadOnly,

    // ── asset ────────────────────────────────────────────────────────────
    /// An import whose bytes no header parser recognised. Its own code because
    /// it is the one asset failure a user can do something about — convert the
    /// file — and a generic "invalid" would not say that.
    #[serde(rename = "asset.not_an_image")]
    NotAnImage,
    /// A link named an asset, or a link, that is not there. Its own code rather
    /// than `node.not_found` because the UI's answer differs: this one means
    /// "the picture panel you are looking at is stale", not "this entity is
    /// gone", and the surfaces that raise it are different.
    #[serde(rename = "asset.not_found")]
    NoSuchAsset,

    // ── share ────────────────────────────────────────────────────────────
    /// The folder went away underneath us — an unmounted share, usually.
    /// Distinct from `io.failed` because it is the one I/O failure that is
    /// expected, recoverable, and worth a banner rather than a toast.
    #[serde(rename = "share.unmounted")]
    ShareUnmounted,

    // ── provider ─────────────────────────────────────────────────────────
    //
    // Defined here rather than when the provider crates land, which is the
    // entire point of this taxonomy: `wobu-llm` and `wobu-imagine` should find
    // the codes already waiting rather than invent their own shapes.
    /// No API key configured for the selected provider. Raised by
    /// `enhance_start`, before anything is submitted or spent.
    #[serde(rename = "provider.no_key")]
    ProviderNoKey,
    /// The OS credential store could not be reached: a locked login keyring, a
    /// headless Linux session, no Secret Service on the bus.
    ///
    /// Distinct from `provider.no_key`, which says the user has not set one up,
    /// and from `internal`, which this file reserves for bugs — a locked keyring
    /// is neither. It is raised only when *storing* a key, because a lookup that
    /// finds nothing is an ordinary unconfigured state and `keys.rs` degrades to
    /// it silently rather than putting a dialog in front of someone who has not
    /// asked for a provider yet.
    #[serde(rename = "provider.keychain_unavailable")]
    ProviderKeychainUnavailable,
    /// The key is present and rejected.
    #[serde(rename = "provider.bad_key")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderBadKey,
    /// The signature was rejected as expired: this machine's clock has drifted
    /// more than the provider tolerates.
    ///
    /// Its own code rather than `provider.bad_key` even though both are
    /// non-retryable auth failures, because the two send the user to opposite
    /// places. Settings offers "re-enter your key" for a bad key, which is
    /// actively wrong advice here — the key is fine and the fix is in the
    /// operating system's date and time. Only signed-request providers can
    /// raise it; a bearer token has no clock in it.
    #[serde(rename = "provider.clock_skew")]
    #[allow(dead_code)] // constructed once wobu-imagine's Tencent adapter lands
    ProviderClockSkew,
    /// The account needs credit or a plan before the request will run.
    #[serde(rename = "provider.billing_required")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderBillingRequired,
    /// Slow down — worth retrying.
    #[serde(rename = "provider.rate_limited")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderRateLimited,
    /// The provider is unreachable or returned a 5xx.
    #[serde(rename = "provider.unavailable")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderUnavailable,
    /// The call succeeded and the answer is unusable — truncated mid-stream,
    /// not JSON, or JSON that does not satisfy the schema it was given.
    ///
    /// Separate from `provider.unavailable` because that one says the service
    /// is down and this one says the model answered badly, and folding them
    /// together tells a user to check their network over a request that
    /// arrived fine. Retryable: the same prompt usually succeeds on a second
    /// attempt, which is exactly what makes it worth telling apart.
    #[serde(rename = "provider.bad_response")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderBadResponse,
    /// The request does not fit the model's context window.
    ///
    /// The one provider failure that is never worth retrying — the stack is
    /// too big by construction, and a "Try again" would burn the user's money
    /// to fail identically. `wobu-influence`'s budget (#43) is the fix, so the
    /// code exists to tell the user which lever to pull.
    #[serde(rename = "provider.context_too_long")]
    #[allow(dead_code)] // constructed once wobu-llm/wobu-imagine land; see below
    ProviderContextTooLong,

    // ── generic ──────────────────────────────────────────────────────────
    /// The user stopped a long operation. Not a failure, and the UI shows
    /// nothing at all for it — a toast saying "cancelled" after you pressed
    /// Cancel is the app arguing with you.
    #[serde(rename = "cancelled")]
    Cancelled,
    /// Filesystem trouble that is not one of the named cases above.
    #[serde(rename = "io.failed")]
    Io,
    /// The local index, or anything with no better home. Always a bug.
    #[serde(rename = "internal")]
    Internal,
}

impl Code {
    /// The dotted string the webview sees, for putting in the log.
    ///
    /// Read back out of the serde attribute rather than duplicated in a match,
    /// so a renamed code cannot say one thing in the log and another on the
    /// bridge. `Code` is a plain unit-variant enum, so this cannot fail.
    pub fn as_str(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "internal".to_owned())
    }

    /// Whether repeating the same request could plausibly succeed.
    ///
    /// Read narrowly: this drives a "Try again" affordance, so a `true` that
    /// leads to the same failure is worse than a missing button. A conflict is
    /// *not* retryable — retrying would produce a second conflict file, and
    /// what it needs is a decision.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Code::Io
                | Code::ShareUnmounted
                | Code::ProviderRateLimited
                | Code::ProviderUnavailable
                | Code::ProviderBadResponse
        )
    }
}

/// The single shape that crosses the bridge.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WobuError {
    pub code: Code,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub retryable: bool,
    /// Only set on `write.conflict`: where our version was parked, relative to
    /// the project root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_path: Option<String>,
}

impl WobuError {
    /// The only constructor, and therefore the only place redaction has to be
    /// remembered. Everything else on this type routes through here.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        let message = redact::scrub(&message.into());
        // Constructing an error is the same event as the user seeing one, so
        // this is the one place that catches every failure for the log without
        // a `diag::error` at each call site — the same argument as redaction.
        diag::error(format!("{}: {message}", code.as_str()));
        WobuError { code, message, detail: None, retryable: code.retryable(), conflict_path: None }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail = redact::scrub(&detail.into());
        // A separate line rather than part of the one above, because the detail
        // is the OS's own wording and is often the only thing in the log that
        // says *why*. At debug so a quiet log still carries the summary.
        diag::record(diag::Level::Debug, format!("  detail: {detail}"));
        self.detail = Some(detail);
        self
    }

    pub fn no_project_open() -> Self {
        WobuError::new(Code::NoProjectOpen, "No project is open.")
    }

    /// A save that lost the race.
    ///
    /// `conflict_path` is the sibling their text was parked in, and it is what
    /// the frontend uses to raise the right conflict card rather than making
    /// the user find it in a list.
    pub fn conflict(conflict_path: String) -> Self {
        let mut e = WobuError::new(
            Code::Conflict,
            format!(
                "Someone else changed this node while you were editing it. \
                 Your version was saved alongside theirs as {conflict_path}."
            ),
        );
        e.conflict_path = Some(conflict_path);
        e
    }
}

impl From<StoreError> for WobuError {
    fn from(e: StoreError) -> Self {
        // `to_string()` up front: every variant already carries written-out
        // `#[error(...)]` copy, and restating it here would leave two versions
        // to drift apart.
        let message = e.to_string();
        let code = match &e {
            StoreError::NotAProject(_) => Code::NotAProject,
            StoreError::AlreadyExists(_) => Code::AlreadyExists,
            StoreError::SchemaTooNew { .. } => Code::SchemaTooNew,
            StoreError::NoProjectOpen => Code::NoProjectOpen,
            StoreError::NoSuchNode(_) => Code::NoSuchNode,
            StoreError::NoSuchAsset(_) | StoreError::NoSuchAssetLink { .. } => Code::NoSuchAsset,
            // A resolution that named something other than a conflict sibling
            // is a bug on the calling side, not a decision the user got wrong,
            // so it lands in the same bucket as any other rejected argument.
            StoreError::NotAConflict(_) => Code::Invalid,
            StoreError::Malformed { .. } | StoreError::MissingFrontmatter(_) => Code::Malformed,
            // An animation shares the code and not the sentence. `NotAnImage`
            // is what the webview already switches on to put a refused drop
            // back on the drop target rather than in an error toast, and the
            // two refusals want exactly the same handling there — what differs
            // is the wording, which travels in `message` and needs no new code
            // on the far side to be read.
            StoreError::NotAnImage | StoreError::AnimatedImage => Code::NotAnImage,
            // A blob already in the library whose pixels will not come back
            // out — almost always one a sync client has not finished copying.
            // `Malformed` rather than `NotAnImage`, because nothing was dropped
            // on a drop target and there is no import to put back: the file is
            // in the folder and it is the *contents* that are wrong.
            StoreError::Undecodable { .. } => Code::Malformed,
            StoreError::ReadOnly => Code::ReadOnly,
            StoreError::Disconnected => Code::ShareUnmounted,
            StoreError::Cancelled => Code::Cancelled,
            StoreError::Core(_) => Code::Invalid,
            // The one I/O case worth telling apart. A share that unmounts
            // mid-session reports `NotFound`/`NotConnected` on every path
            // under it, and #20 wants that as a banner with backoff — not as
            // the same toast a failed thumbnail write gets.
            StoreError::Io { source, .. } if is_gone(source) => Code::ShareUnmounted,
            StoreError::Io { .. } => Code::Io,
            StoreError::Yaml(_) | StoreError::Json(_) | StoreError::Sqlite(_) => Code::Internal,
        };

        let error = WobuError::new(code, message);
        // The source chain is where the operating system's own wording lives.
        // Useful in a diagnostics log, too noisy for a toast.
        match &e {
            StoreError::Io { source, .. } => error.with_detail(source.to_string()),
            StoreError::Yaml(inner) => error.with_detail(inner.to_string()),
            StoreError::Json(inner) => error.with_detail(inner.to_string()),
            StoreError::Sqlite(inner) => error.with_detail(inner.to_string()),
            _ => error,
        }
    }
}

impl From<wobu_core::Error> for WobuError {
    fn from(e: wobu_core::Error) -> Self {
        WobuError::new(Code::Invalid, e.to_string())
    }
}

/// Whether an I/O error means "the thing this path lives on is not there",
/// as opposed to "this particular operation failed".
fn is_gone(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        e.kind(),
        ErrorKind::NotFound
            | ErrorKind::NotConnected
            | ErrorKind::HostUnreachable
            | ErrorKind::BrokenPipe
    )
}

pub type CommandResult<T> = std::result::Result<T, WobuError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn json(e: &WobuError) -> serde_json::Value {
        serde_json::to_value(e).unwrap()
    }

    #[test]
    fn codes_serialise_as_the_dotted_strings_the_frontend_switches_on() {
        // These strings are load-bearing: `src/lib/api.ts` compares against
        // them literally. Renaming a variant must not silently change one.
        let cases = [
            (Code::NotAProject, "project.not_a_project"),
            (Code::SchemaTooNew, "project.schema_too_new"),
            (Code::NoProjectOpen, "project.none_open"),
            (Code::NoSuchNode, "node.not_found"),
            (Code::Invalid, "node.invalid"),
            (Code::Conflict, "write.conflict"),
            (Code::ReadOnly, "write.read_only"),
            (Code::NotAnImage, "asset.not_an_image"),
            (Code::NoSuchAsset, "asset.not_found"),
            (Code::ShareUnmounted, "share.unmounted"),
            (Code::ProviderNoKey, "provider.no_key"),
            (Code::ProviderKeychainUnavailable, "provider.keychain_unavailable"),
            (Code::ProviderBillingRequired, "provider.billing_required"),
            (Code::Io, "io.failed"),
            (Code::Internal, "internal"),
        ];
        for (code, expected) in cases {
            assert_eq!(serde_json::to_value(code).unwrap(), expected);
        }
    }

    #[test]
    fn retryable_is_true_only_where_trying_again_could_work() {
        assert!(Code::ShareUnmounted.retryable());
        assert!(Code::ProviderRateLimited.retryable());
        assert!(Code::Io.retryable());
        // The one this list was actually missing. `wobu-llm` and `wobu-imagine`
        // both classify a truncated or malformed answer as retryable and the
        // queue acts on that, so omitting it here meant the queue quietly
        // retrying a failure the UI was simultaneously refusing to offer a
        // "Try again" for — the two halves of one decision disagreeing.
        assert!(Code::ProviderBadResponse.retryable());

        // A second attempt makes a second conflict file, not a resolution.
        assert!(!Code::Conflict.retryable());
        assert!(!Code::ReadOnly.retryable());
        // Dropping the same PDF again produces the same PDF.
        assert!(!Code::NotAnImage.retryable());
        // The id is derived from a hash, so it will not start matching a file.
        assert!(!Code::NoSuchAsset.retryable());
        assert!(!Code::Invalid.retryable());
        assert!(!Code::ProviderNoKey.retryable());
        // A locked keyring stays locked until the user unlocks it, so a "Try
        // again" here would fail identically and teach them to distrust it.
        assert!(!Code::ProviderKeychainUnavailable.retryable());
    }

    #[test]
    fn a_key_cannot_reach_the_webview_through_any_field() {
        // The constructor is the only way in, so this covers every crate.
        let e =
            WobuError::new(Code::ProviderBadKey, "401 from x-api-key: sk-ant-api03-leakleakleak")
                .with_detail("retried with Authorization: Bearer sk-ant-api03-leakleakleak");

        let serialised = json(&e).to_string();
        assert!(!serialised.contains("sk-ant-api03-leakleakleak"), "{serialised}");
        assert!(!serialised.contains("leakleakleak"), "{serialised}");
        assert!(serialised.contains("401"), "still diagnosable: {serialised}");
    }

    #[test]
    fn a_conflict_carries_its_path_and_says_so_in_words() {
        let e = WobuError::conflict("nodes/species/vashk.conflict.md".into());
        let j = json(&e);

        assert_eq!(j["code"], "write.conflict");
        assert_eq!(j["conflictPath"], "nodes/species/vashk.conflict.md");
        assert_eq!(j["retryable"], false);
        // `errorMessage()` in api.ts reads `.message` and nothing else, so a
        // conflict must be legible without the code being understood.
        assert!(e.message.contains("nodes/species/vashk.conflict.md"));
    }

    #[test]
    fn optional_fields_are_omitted_rather_than_null() {
        // TypeScript reads these as `detail?: string`, so `null` would be a
        // different type from absent.
        let j = json(&WobuError::no_project_open());
        assert!(j.get("detail").is_none(), "{j}");
        assert!(j.get("conflictPath").is_none(), "{j}");
    }

    #[test]
    fn store_errors_keep_their_own_wording() {
        let e: WobuError = StoreError::NoSuchNode("01ARZ3NDEKTSV4RRFFQ69G5FAV".into()).into();
        assert_eq!(json(&e)["code"], "node.not_found");
        assert_eq!(e.message, "no node with id 01ARZ3NDEKTSV4RRFFQ69G5FAV");

        let e: WobuError = StoreError::Core(wobu_core::Error::SelfParent).into();
        assert_eq!(json(&e)["code"], "node.invalid");
        assert_eq!(e.message, "a node cannot be its own parent");
    }

    #[test]
    fn a_disconnected_share_reaches_the_ui_as_a_retryable_banner_code() {
        // `Error::Disconnected` is what `reconcile` and every write path raise
        // once the folder stops being reachable. It has to arrive as the code
        // `errorSurface` routes to a banner, and as retryable — the share
        // coming back is the expected outcome.
        let e: WobuError = StoreError::Disconnected.into();
        let j = json(&e);
        assert_eq!(j["code"], "share.unmounted");
        assert_eq!(j["retryable"], true);
        assert!(e.message.contains("unmounted"), "{}", e.message);
    }

    #[test]
    fn an_unmounted_share_is_told_apart_from_ordinary_io_trouble() {
        // #20 hangs a banner and a backoff off this distinction, so it has to
        // survive the flattening.
        let gone = StoreError::io(
            PathBuf::from("/mnt/art/Ashfall.wobu/nodes/species/vashk.md"),
            std::io::Error::from(std::io::ErrorKind::NotConnected),
        );
        let e: WobuError = gone.into();
        assert_eq!(json(&e)["code"], "share.unmounted");
        assert_eq!(json(&e)["retryable"], true);

        let full = StoreError::io(
            PathBuf::from("/home/art/Ashfall.wobu/nodes/species/vashk.md"),
            std::io::Error::from(std::io::ErrorKind::StorageFull),
        );
        let e: WobuError = full.into();
        assert_eq!(json(&e)["code"], "io.failed");
    }

    #[test]
    fn the_os_wording_lands_in_detail_not_in_the_message() {
        let e: WobuError = StoreError::io(
            PathBuf::from("/mnt/art/Ashfall.wobu"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        )
        .into();

        assert!(e.detail.is_some(), "the source chain should be preserved");
        assert!(e.detail.unwrap().to_lowercase().contains("permission"));
    }
}
