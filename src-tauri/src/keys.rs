//! Where a provider's key comes from, and what the rest of the process is
//! allowed to know about it.
//!
//! Keys are per *installation*, never per project. A project folder is meant to
//! be put on a share (`docs/07-file-shares.md`), so a key written into
//! `project.json` is a key handed to everyone with the path — and to git
//! history, and to whoever the folder gets zipped to. `project.json` therefore
//! carries only the *selection*: provider id, model id, default params. The key
//! for `wobu/<provider>` is looked up here, on this machine, which is what makes
//! opening someone else's world use your own credentials rather than theirs.
//!
//! ## What crosses the bridge
//!
//! [`KeyStatus`], and nothing else. It answers "is there a key, and where did it
//! come from" — enough for the UI to render "Gemini selected — no key on this
//! machine" beside a direct affordance to add one, and not enough for anything
//! else. The key itself is a [`Secret`]: no `Serialize`, no `Display`, and a
//! hand-written `Debug` that prints the mask. A `#[derive(Debug)]` on a struct
//! that happens to hold a `String` key is the ordinary way a credential reaches
//! a log, and [`Secret`] is the arrangement that makes that derive harmless.
//!
//! Key material travels *in* from the webview — the user pastes one into a
//! field — and never back out. That asymmetry is the whole design, and
//! `provider_key_set` in `commands.rs` is the only door it goes through.
//!
//! ## An absent or locked keychain is normal
//!
//! Headless Linux, a CI box, a session whose login keyring has not been
//! unlocked: none of those are errors and none of them stop the app. A store
//! that will not answer degrades to "unconfigured", which is a state the UI
//! already has to render for a provider nobody has set up. The only place it is
//! reported as a failure is a *write*, because silently not saving a key the
//! user just pasted is worse than saying so.
//!
//! See `docs/08-providers.md`.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::Serialize;

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::redact;

/// The service half of the keychain entry, so a provider's key lives at
/// `wobu/<provider>`. The other half is `TextProvider::id`, which documents the
/// same pairing from the adapter's side — renaming an id orphans every key
/// already stored under the old one, on every machine.
const SERVICE: &str = "wobu";

/* ── the secret ───────────────────────────────────────────────────────────── */

/// A credential, in the one shape that cannot be printed by accident.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// `pub(crate)` so that `enhance.rs`'s tests can build an adapter without a
    /// real credential. Nothing outside this crate can mint one, which is the
    /// property that matters: a `Secret` in the wild has come from the keychain
    /// or from the development-time fallback and from nowhere else.
    pub(crate) fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The key itself.
    ///
    /// Named so that reaching for it reads as a claim rather than as a getter:
    /// the caller is about to put a credential somewhere, and the only somewhere
    /// that is allowed is an outbound request header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Hand-written on purpose. A derive would print the key, and every type that
/// holds a `Secret` derives `Debug` — so this is the impl standing between a
/// credential and `{:?}` in a log line, in a panic message, or in the `unwrap`
/// of a `Result` whose error type happens to carry one.
///
/// It prints the same mask [`redact::scrub`] uses, so a line reads identically
/// whether it was scrubbed on the way out or was never a key to begin with.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(redact::MASK)
    }
}

/* ── what the webview is told ─────────────────────────────────────────────── */

/// Where a key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Keychain,
    /// The development-time fallback: a process variable, or the repo-root
    /// `.env`.
    ///
    /// The variant exists in release builds even though nothing there can
    /// produce it. A `cfg` on a serialised type would make the wire format
    /// differ between the build a developer tests and the build a user runs,
    /// which is a worse problem than a variant that never occurs.
    Environment,
}

/// Whether this machine has a credential store that will answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Keychain {
    Ready,
    /// No Secret Service, a locked login keyring, a headless session. Not a
    /// failure: it means keys cannot be *stored* on this machine, and the UI
    /// says so instead of offering a field that will not work.
    Unavailable,
}

/// Presence, never value. The entire surface the webview gets.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub provider: String,
    /// `None` is "no key on this machine", which is a state rather than an
    /// error — a collaborator opening a shared project is in it by default.
    pub source: Option<Source>,
    /// A property of the machine, reported per provider because the pane that
    /// renders one of these is exactly where "you cannot save a key here" has
    /// to be said.
    pub keychain: Keychain,
}

/// What a delete did, and what the provider resolves to afterwards.
///
/// A shape rather than a bare `bool` because two outcomes are easy to confuse
/// and both are ordinary. "There was nothing stored" is not a failure. And on a
/// developer's machine, removing the keychain entry can leave the provider
/// *still configured* — the repo-root `.env` answers next. Returning the fresh
/// status makes that visible in the same round trip instead of as a mystery.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRemoval {
    /// False when no entry existed.
    pub removed: bool,
    pub status: KeyStatus,
}

/* ── the credential store ─────────────────────────────────────────────────── */

/// The store did not answer. One variant, because there is exactly one thing
/// the rest of this module does about it: stop trusting the store and carry on.
/// The string is the platform's own wording, kept for `detail`.
#[derive(Debug)]
struct Unavailable(String);

/// The OS credential store.
///
/// A trait so that the resolution order below can be tested without one. CI has
/// no Secret Service and a developer's session may have a locked one, so a test
/// that reached the real keychain would either be skipped everywhere it matters
/// or would write credentials into the machine running it.
trait Store: Send + Sync {
    /// `Ok(None)` when there is no entry — never set, or deleted.
    fn get(&self, provider: &str) -> Result<Option<Secret>, Unavailable>;
    fn set(&self, provider: &str, secret: &str) -> Result<(), Unavailable>;
    /// `Ok(false)` when there was nothing to remove.
    fn delete(&self, provider: &str) -> Result<bool, Unavailable>;
}

/// Secret Service on Linux, Keychain on macOS, Credential Manager on Windows —
/// which is what `keyring`'s default `v1` feature selects, one per platform.
struct OsStore;

impl OsStore {
    fn entry(provider: &str) -> Result<keyring::Entry, Unavailable> {
        // Fails when there is no usable store at all, which on Linux means no
        // Secret Service on the session bus. Ordinary on a server.
        keyring::Entry::new(SERVICE, provider).map_err(|e| Unavailable(e.to_string()))
    }
}

impl Store for OsStore {
    fn get(&self, provider: &str) -> Result<Option<Secret>, Unavailable> {
        match OsStore::entry(provider)?.get_password() {
            Ok(password) => Ok(Some(Secret::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Unavailable(e.to_string())),
        }
    }

    fn set(&self, provider: &str, secret: &str) -> Result<(), Unavailable> {
        OsStore::entry(provider)?.set_password(secret).map_err(|e| Unavailable(e.to_string()))
    }

    fn delete(&self, provider: &str) -> Result<bool, Unavailable> {
        match OsStore::entry(provider)?.delete_credential() {
            Ok(()) => Ok(true),
            // Deleting a key that is not there is a no-op, not a problem. The
            // distinction is carried out to the UI in `KeyRemoval::removed`,
            // because "removed" and "there was nothing" are different sentences.
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(Unavailable(e.to_string())),
        }
    }
}

/* ── resolution ───────────────────────────────────────────────────────────── */

/// What a resolution found.
///
/// `Debug` is derived and that is safe *because* of [`Secret`]'s own impl —
/// which is the property the redaction test pins, since this is the type a
/// future `{:?}` is most likely to be pointed at.
#[derive(Debug, Clone)]
struct Lookup {
    source: Option<Source>,
    secret: Option<Secret>,
    keychain: Keychain,
}

impl Lookup {
    fn unconfigured(keychain: Keychain) -> Lookup {
        Lookup { source: None, secret: None, keychain }
    }
}

/// How a development-time variable is read.
type ReadVar = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The process-wide answer to "what is this provider's key".
///
/// Managed Tauri state, registered in `lib.rs`. Holds no reference to the open
/// project and never will: a key belongs to the installation, and a type that
/// could see a project folder is a type that could one day read a key out of
/// one.
pub struct Keys {
    store: Box<dyn Store>,
    /// A field rather than a direct call so the fallback can be tested without
    /// the test depending on the environment of whoever runs it — which would
    /// make it pass or fail based on whether the developer happens to have a
    /// real key exported.
    var: ReadVar,
    /// Resolved lookups, by provider.
    ///
    /// Not a speed optimisation. A locked Secret Service prompts the user on
    /// every read, and the generate path asks once per job — so without this the
    /// app would ask for a password every time somebody presses Enhance.
    /// Cleared for a provider whenever this process changes its key; a key
    /// changed by some *other* program is picked up on the next run, which is
    /// the trade being made.
    cache: Mutex<HashMap<String, Lookup>>,
}

impl Default for Keys {
    fn default() -> Keys {
        Keys { store: Box::new(OsStore), var: Box::new(dev_var), cache: Mutex::default() }
    }
}

impl Keys {
    /// The key for a provider, or `None` when this machine has none.
    ///
    /// The only way a [`Secret`] leaves this module, and it goes to an adapter
    /// constructor — `AnthropicProvider::new(key)` — rather than into a struct
    /// the UI can see.
    pub fn secret(&self, provider: &str) -> Option<Secret> {
        self.lookup(provider).secret
    }

    /// Everything the UI is allowed to know.
    pub fn status(&self, provider: &str) -> KeyStatus {
        let found = self.lookup(provider);
        KeyStatus { provider: provider.to_owned(), source: found.source, keychain: found.keychain }
    }

    /// Store a key for a provider, replacing any entry already there.
    pub fn set(&self, provider: &str, key: &str) -> CommandResult<KeyStatus> {
        // A pasted key arrives with a trailing newline more often than not, and
        // a key with one on the end is a 401 that reads exactly like a wrong
        // key — which sends the user back to the console for a replacement that
        // will fail the same way.
        let key = key.trim();
        if key.is_empty() {
            return Err(WobuError::new(Code::Invalid, "A key cannot be empty."));
        }

        self.store.set(provider, key).map_err(|e| unavailable("Your key was not saved.", e))?;
        self.forget(provider);
        Ok(self.status(provider))
    }

    /// Remove this machine's stored key for a provider.
    pub fn delete(&self, provider: &str) -> CommandResult<KeyRemoval> {
        let removed =
            self.store.delete(provider).map_err(|e| unavailable("The key was not removed.", e))?;
        self.forget(provider);
        Ok(KeyRemoval { removed, status: self.status(provider) })
    }

    /// Keychain, then environment, then unconfigured — and that order is the
    /// specification rather than an implementation detail. The keychain winning
    /// is what lets a developer keep a `.env` around while still exercising the
    /// path a user is on.
    fn lookup(&self, provider: &str) -> Lookup {
        let cached = self.cache.lock().get(provider).cloned();
        if let Some(hit) = cached {
            return hit;
        }

        let (keychain, stored) = match self.store.get(provider) {
            Ok(found) => (Keychain::Ready, found),
            Err(e) => {
                // Info, not error. A machine without a credential store is an
                // ordinary machine, and an ERROR line here would be the first
                // thing a reader sees in a log sent about something else.
                diag::info(format!("no credential store for {provider}: {}", e.0));
                (Keychain::Unavailable, None)
            }
        };

        let found = match stored {
            Some(secret) => {
                Lookup { source: Some(Source::Keychain), secret: Some(secret), keychain }
            }
            None => match self.env_secret(provider) {
                Some(secret) => {
                    Lookup { source: Some(Source::Environment), secret: Some(secret), keychain }
                }
                None => Lookup::unconfigured(keychain),
            },
        };

        // Only a definitive answer is remembered. An unreachable store is the
        // one result worth asking about again: the user's next move after
        // reading "credential store unavailable" is to unlock their keyring,
        // and a cached `Unavailable` would keep saying no afterwards.
        if keychain == Keychain::Ready {
            self.cache.lock().insert(provider.to_owned(), found.clone());
        }
        found
    }

    fn env_secret(&self, provider: &str) -> Option<Secret> {
        for name in env_names(provider) {
            let Some(value) = (self.var)(&name) else { continue };
            let value = value.trim();
            // An empty variable is what `.env.example` ships, and treating it as
            // a key would report a provider as configured and then fail every
            // request with a 401.
            if !value.is_empty() {
                return Some(Secret::new(value));
            }
        }
        None
    }

    fn forget(&self, provider: &str) {
        self.cache.lock().remove(provider);
    }
}

/// The one failure this module reports to a person.
///
/// Not `Code::Internal`: `error.rs` reserves that for bugs, and a locked login
/// keyring is not one. Not retryable either — pressing "Try again" without
/// unlocking anything fails identically, so the instruction goes in the message
/// where the user can act on it instead of behind a button that repeats itself.
fn unavailable(what_happened: &str, e: Unavailable) -> WobuError {
    WobuError::new(
        Code::ProviderKeychainUnavailable,
        format!(
            "{what_happened} This computer's credential store did not answer. \
             On Linux that usually means the login keyring is locked."
        ),
    )
    .with_detail(e.0)
}

/* ── the development-time fallback ────────────────────────────────────────── */

/// Variable names to try for a provider, in order.
///
/// The convention is the provider id upper-cased with `_API_KEY` on the end, so
/// an adapter added later works without a line here. The table is only for
/// credentials that were named something else before Wobu existed.
fn env_names(provider: &str) -> Vec<String> {
    match provider {
        // Tencent's credential is a SecretId/SecretKey *pair* signed with
        // TC3-HMAC-SHA256, not a bearer token. It is registered as two
        // credentials rather than one because there is no join format for a
        // pair that this module and an adapter would both have to agree on, and
        // inventing one would put a parser between a key and a request
        // signature — where a mis-split is an auth failure with no explanation.
        "tencent-secret-id" => vec!["TencentSecretId".to_owned()],
        "tencent-secret-key" => vec!["TencentSecretKey".to_owned()],
        _ => vec![format!("{}_API_KEY", provider.to_ascii_uppercase().replace('-', "_"))],
    }
}

/// A development-time variable: the process environment first, then the
/// repo-root `.env`.
///
/// Compiled out of release builds entirely, and the `.env` reader below with it.
/// A shipped Wobu reads credentials from the keychain and from nowhere else,
/// because the moment a *file* can supply a key the next question is which file
/// — and the answer a user would reach for is one inside their project folder,
/// on the share, which is the leak the keychain rule exists to prevent.
fn dev_var(name: &str) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(name) {
            return Some(value);
        }
        std::fs::read_to_string(dev_env_path()).ok().and_then(|text| dot_env(&text, name))
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        None
    }
}

/// The repo-root `.env`, and only ever that one.
///
/// Fixed at *compile* time from `CARGO_MANIFEST_DIR`, which is the property that
/// matters rather than the convenience: there is no argument, no working
/// directory and no setting that can point this at a project folder. A project
/// folder lives on a share, and a key in one is a key handed to everyone with
/// the path — so the dev-time exception is not allowed to reopen the hole the
/// keychain rule was written to close.
#[cfg(debug_assertions)]
fn dev_env_path() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.env"))
}

/// `NAME=value` out of a `.env`, quotes and `export` and comments allowed.
///
/// Hand-rolled rather than `dotenvy` for the same reason `diag.rs` does not take
/// `tracing`: this is one pass over a handful of lines, it is compiled out of
/// the shipped binary, and a dotenv crate's usual job — mutating the process
/// environment through `set_var` — is something this module specifically does
/// not want to do.
#[cfg(debug_assertions)]
fn dot_env(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export FOO=bar` is what a `.env` that also gets sourced looks like.
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != name {
            continue;
        }
        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
        return Some(unquoted.unwrap_or(value).to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store that lives in memory, or refuses to answer at all.
    ///
    /// `fails_with` carries a made-up platform message; one test loads it with a
    /// key, because a platform error that echoes the failing request is exactly
    /// how a credential ends up in `detail`.
    #[derive(Default)]
    struct FakeStore {
        entries: Mutex<HashMap<String, String>>,
        fails_with: Option<String>,
    }

    impl FakeStore {
        fn refusing(message: &str) -> FakeStore {
            FakeStore { fails_with: Some(message.to_owned()), ..FakeStore::default() }
        }

        fn holding(provider: &str, key: &str) -> FakeStore {
            let store = FakeStore::default();
            store.entries.lock().insert(provider.to_owned(), key.to_owned());
            store
        }

        fn refuse(&self) -> Option<Unavailable> {
            self.fails_with.as_ref().map(|m| Unavailable(m.clone()))
        }
    }

    impl Store for FakeStore {
        fn get(&self, provider: &str) -> Result<Option<Secret>, Unavailable> {
            if let Some(e) = self.refuse() {
                return Err(e);
            }
            Ok(self.entries.lock().get(provider).map(Secret::new))
        }

        fn set(&self, provider: &str, secret: &str) -> Result<(), Unavailable> {
            if let Some(e) = self.refuse() {
                return Err(e);
            }
            self.entries.lock().insert(provider.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, provider: &str) -> Result<bool, Unavailable> {
            if let Some(e) = self.refuse() {
                return Err(e);
            }
            Ok(self.entries.lock().remove(provider).is_some())
        }
    }

    /// `Keys` over a fake store and a fixed environment, so nothing in this
    /// module's tests touches the machine running them.
    fn keys(store: FakeStore, env: &[(&str, &str)]) -> Keys {
        let env: HashMap<String, String> =
            env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Keys {
            store: Box::new(store),
            var: Box::new(move |name| env.get(name).cloned()),
            cache: Mutex::default(),
        }
    }

    fn json(value: &impl Serialize) -> serde_json::Value {
        serde_json::to_value(value).unwrap()
    }

    /* ── the test #31 asks for ────────────────────────────────────────────── */

    /// The test the issue names, written wide on purpose.
    ///
    /// A key that reaches the webview does so through whichever field nobody
    /// thought about, so asserting on `error.message` alone would pass while the
    /// key travelled in `detail`. This covers every route out of the process:
    /// the `Debug` of the type holding the key, the serialised JSON that
    /// actually crosses the bridge, and the log file on disk.
    #[test]
    fn a_key_never_appears_in_a_serialised_error_a_debug_dump_or_a_log_line() {
        const KEY: &str = "sk-ant-api03-leakleakleakleak";

        // 1. The `Debug` impl of the type that holds a key. A derive here is the
        //    classic leak, and every enclosing type derives `Debug`.
        let secret = Secret::new(KEY);
        assert_eq!(format!("{secret:?}"), redact::MASK);
        let holder = Lookup {
            source: Some(Source::Keychain),
            secret: Some(secret),
            keychain: Keychain::Ready,
        };
        let dumped = format!("{holder:?}");
        assert!(!dumped.contains(KEY), "a key survived a struct's Debug: {dumped}");
        assert!(dumped.contains("Keychain"), "the dump is still useful: {dumped}");

        // 2. The serialised error, not just its `Display` — this is the JSON the
        //    webview receives. A provider that echoes the failing request is the
        //    realistic source of both of these strings.
        let error = WobuError::new(
            Code::ProviderBadKey,
            format!("GET https://api.anthropic.com/v1/models?key={KEY} returned 401"),
        )
        .with_detail(format!("sent header x-api-key: {KEY}"));
        let serialised = json(&error).to_string();
        assert!(!serialised.contains(KEY), "a key crossed the bridge: {serialised}");
        assert!(!serialised.contains("leakleakleak"), "{serialised}");
        assert!(serialised.contains("401"), "still diagnosable: {serialised}");

        // 3. The failure this module raises itself, where the platform's own
        //    wording lands in `detail` unread.
        let refusing = keys(FakeStore::refusing(&format!("store rejected api_key={KEY}")), &[]);
        let refused = refusing.set("anthropic", KEY).expect_err("the store refused");
        let serialised = json(&refused).to_string();
        assert!(!serialised.contains(KEY), "a key reached the webview in an error: {serialised}");

        // 4. Everything this module hands the UI, whatever a future field on it
        //    might hold.
        let configured = keys(FakeStore::holding("anthropic", KEY), &[]);
        let status = json(&configured.status("anthropic")).to_string();
        assert!(!status.contains(KEY), "a key reached the webview in a status: {status}");
        let removal = json(&configured.delete("anthropic").unwrap()).to_string();
        assert!(!removal.contains(KEY), "a key reached the webview in a removal: {removal}");

        // 5. The log on disk, which is the file a user pastes into an issue.
        let dir = std::env::temp_dir().join(format!("wobu-keys-redact-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = diag::Diagnostics::new(dir.clone());
        log.record(diag::Level::Error, &format!("POST failed, Authorization: Bearer {KEY}"));
        log.record(diag::Level::Error, &format!("no credential store: api_key={KEY}"));
        let written = std::fs::read_to_string(log.path()).unwrap_or_default();
        assert!(!written.contains(KEY), "a key was written to the log: {written}");
        assert!(written.contains(redact::MASK), "nothing was masked: {written}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_status_carries_presence_and_has_no_field_that_could_carry_a_value() {
        // The regression this really guards is a *new* field on `KeyStatus`.
        // Whoever adds one has to come here and say what it is, which is the
        // moment to notice that it holds key material.
        let configured = keys(FakeStore::holding("anthropic", "sk-ant-api03-real"), &[]);
        let status = json(&configured.status("anthropic"));

        let mut fields: Vec<&str> =
            status.as_object().expect("an object").keys().map(String::as_str).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["keychain", "provider", "source"], "{status}");

        assert_eq!(status["provider"], "anthropic");
        assert_eq!(status["source"], "keychain");
        assert_eq!(status["keychain"], "ready");
    }

    /* ── resolution ───────────────────────────────────────────────────────── */

    #[test]
    fn resolution_is_keychain_then_environment_then_unconfigured() {
        // The order is the spec in `docs/08-providers.md`. The keychain winning
        // is what lets a developer keep a `.env` on disk and still exercise the
        // path a user is actually on.
        let both = keys(
            FakeStore::holding("anthropic", "from-keychain"),
            &[("ANTHROPIC_API_KEY", "from-env")],
        );
        assert_eq!(both.status("anthropic").source, Some(Source::Keychain));
        assert_eq!(both.secret("anthropic").unwrap().expose(), "from-keychain");

        let env_only = keys(FakeStore::default(), &[("ANTHROPIC_API_KEY", "from-env")]);
        assert_eq!(env_only.status("anthropic").source, Some(Source::Environment));
        assert_eq!(env_only.secret("anthropic").unwrap().expose(), "from-env");

        let neither = keys(FakeStore::default(), &[]);
        assert_eq!(neither.status("gemini").source, None);
        assert!(neither.secret("gemini").is_none());
    }

    #[test]
    fn an_absent_keychain_is_unconfigured_rather_than_an_error() {
        // Headless Linux, a CI box, a session whose keyring is locked. Reading a
        // key has no `Result` at all, so there is no path from here to a dialog.
        let headless = keys(FakeStore::refusing("no Secret Service on the session bus"), &[]);

        let status = headless.status("gemini");
        assert_eq!(status.keychain, Keychain::Unavailable);
        assert_eq!(status.source, None);
        assert!(headless.secret("gemini").is_none());
    }

    #[test]
    fn a_developer_fallback_still_works_with_no_keychain_at_all() {
        // The CI shape: no credential store, a key in the environment. It has to
        // resolve, or every provider test on a build box is unconfigured.
        let ci = keys(
            FakeStore::refusing("no Secret Service on the session bus"),
            &[("ANTHROPIC_API_KEY", "from-env")],
        );
        let status = ci.status("anthropic");
        assert_eq!(status.keychain, Keychain::Unavailable);
        assert_eq!(status.source, Some(Source::Environment));
    }

    #[test]
    fn an_unreachable_store_is_asked_again_rather_than_remembered() {
        // The user's move after reading "credential store did not answer" is to
        // unlock their keyring. A cached `Unavailable` would keep saying no for
        // the rest of the session and look like the unlock did nothing.
        let store = FakeStore::refusing("locked");
        let mut keys = keys(store, &[]);
        assert_eq!(keys.status("anthropic").keychain, Keychain::Unavailable);

        keys.store = Box::new(FakeStore::holding("anthropic", "unlocked-now"));
        assert_eq!(keys.status("anthropic").keychain, Keychain::Ready);
        assert_eq!(keys.status("anthropic").source, Some(Source::Keychain));
    }

    /* ── writing and removing ─────────────────────────────────────────────── */

    #[test]
    fn deleting_a_key_is_told_apart_from_there_never_having_been_one() {
        let keys = keys(FakeStore::holding("anthropic", "sk-ant-api03-real"), &[]);

        let first = keys.delete("anthropic").unwrap();
        assert!(first.removed, "an entry existed and should report as removed");
        assert_eq!(first.status.source, None);

        let second = keys.delete("anthropic").unwrap();
        assert!(!second.removed, "a second delete removed nothing and must say so");
        // And it is still not a failure — the UI must not raise anything for it.
        assert_eq!(second.status.source, None);
    }

    #[test]
    fn deleting_the_stored_key_does_not_delete_the_developer_fallback() {
        // The confusing case this exists for: on a developer's machine the
        // provider is still configured after a delete, because the repo `.env`
        // answers next. Reporting the fresh status is what makes that legible
        // rather than a mystery.
        let keys = keys(
            FakeStore::holding("anthropic", "from-keychain"),
            &[("ANTHROPIC_API_KEY", "from-env")],
        );

        let removal = keys.delete("anthropic").unwrap();
        assert!(removal.removed);
        assert_eq!(removal.status.source, Some(Source::Environment));
    }

    #[test]
    fn a_stored_key_replaces_whatever_was_there_and_is_visible_at_once() {
        // The cache is the thing being guarded: a save that the next read does
        // not see would show the user their old state and read as a failed save.
        let keys = keys(FakeStore::default(), &[]);
        assert_eq!(keys.status("anthropic").source, None);

        let status = keys.set("anthropic", "sk-ant-api03-first").unwrap();
        assert_eq!(status.source, Some(Source::Keychain));
        assert_eq!(keys.secret("anthropic").unwrap().expose(), "sk-ant-api03-first");

        keys.set("anthropic", "sk-ant-api03-second").unwrap();
        assert_eq!(keys.secret("anthropic").unwrap().expose(), "sk-ant-api03-second");
    }

    #[test]
    fn a_pasted_key_loses_the_whitespace_it_was_pasted_with() {
        // A key with a trailing newline is a 401 that reads exactly like a wrong
        // key, which sends the user back to the console for a replacement that
        // fails identically.
        let keys = keys(FakeStore::default(), &[]);
        keys.set("anthropic", "  sk-ant-api03-pasted\n").unwrap();
        assert_eq!(keys.secret("anthropic").unwrap().expose(), "sk-ant-api03-pasted");
    }

    #[test]
    fn an_empty_key_is_refused_rather_than_stored() {
        // Storing one would report the provider as configured and then fail
        // every request, which is the worst of both states.
        let keys = keys(FakeStore::default(), &[]);
        let e = keys.set("anthropic", "   \n").expect_err("an empty key is not a key");
        assert_eq!(json(&e)["code"], "node.invalid");
        assert_eq!(keys.status("anthropic").source, None);
    }

    #[test]
    fn an_empty_environment_variable_is_not_a_key() {
        // `.env.example` ships every variable empty, and a developer who copies
        // it without filling anything in must read as unconfigured rather than
        // as configured-and-broken.
        let keys = keys(FakeStore::default(), &[("ANTHROPIC_API_KEY", "  ")]);
        assert_eq!(keys.status("anthropic").source, None);
    }

    #[test]
    fn failing_to_save_is_reported_rather_than_silently_dropped() {
        // The one place an unusable credential store is an error: the user
        // pasted a key and pressed Save, and pretending that worked is worse
        // than a dialog.
        let keys = keys(FakeStore::refusing("the collection is locked"), &[]);
        let e = keys.set("anthropic", "sk-ant-api03-real").expect_err("the store refused");

        assert_eq!(json(&e)["code"], "provider.keychain_unavailable");
        // Not retryable: pressing "Try again" without unlocking anything fails
        // identically, so the instruction is in the message instead.
        assert_eq!(json(&e)["retryable"], false);
        assert!(e.message.contains("credential store"), "{}", e.message);
    }

    /* ── the development-time fallback ────────────────────────────────────── */

    #[test]
    fn the_env_file_is_the_repo_root_and_can_never_be_inside_a_project() {
        // The rule the whole keychain design exists to protect: a `.env` read
        // from a project folder would be a key on a share. Two separate claims
        // are being made, and the second is the one that matters.
        let path = dev_env_path();

        // 1. It points where it is meant to. Resolved against this crate's own
        //    manifest directory rather than against a spelled-out path, so the
        //    assertion still means "the repo root" if the tree is checked out
        //    somewhere else or the shell crate is renamed.
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some(".env"), "{}", path.display());
        let shell = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = path.parent().expect("a parent").canonicalize().expect("the repo root exists");
        assert_eq!(root, shell.parent().expect("a parent above src-tauri"));
        assert!(
            root.join("src-tauri/Cargo.toml").is_file(),
            "not the repo root: {}",
            root.display()
        );
        assert!(root.join(".env.example").is_file(), "the documented names live here");

        // 2. It cannot be redirected. `dev_env_path` takes no argument and reads
        //    no variable, so the only input is `CARGO_MANIFEST_DIR`, which is
        //    fixed when this crate is compiled — there is nothing a caller, a
        //    working directory or a setting could supply to point it at a
        //    project folder. Restated as an assertion because the failure it
        //    guards against is silent: a `.env` inside a `.wobu` on a share is a
        //    key published to everyone who can mount it.
        assert!(
            !path.components().any(|c| c.as_os_str().to_string_lossy().ends_with(".wobu")),
            "the dev fallback pointed inside a project folder: {}",
            path.display()
        );
    }

    #[test]
    fn env_variable_names_follow_the_id_except_where_the_vendor_named_them_first() {
        assert_eq!(env_names("anthropic"), ["ANTHROPIC_API_KEY"]);
        assert_eq!(env_names("gemini"), ["GEMINI_API_KEY"]);
        // A provider nobody has written yet resolves without a line in the
        // table, which is the point of deriving rather than enumerating.
        assert_eq!(env_names("some-new-vendor"), ["SOME_NEW_VENDOR_API_KEY"]);
        // Tencent's is a signed SecretId/SecretKey pair, so it is two
        // credentials rather than one joined string.
        assert_eq!(env_names("tencent-secret-id"), ["TencentSecretId"]);
        assert_eq!(env_names("tencent-secret-key"), ["TencentSecretKey"]);
    }

    #[test]
    fn the_env_file_reader_handles_what_a_real_dotenv_looks_like() {
        let text = "\
# a comment
ANTHROPIC_API_KEY=sk-ant-api03-plain

export GEMINI_API_KEY = \"quoted-and-exported\"
TencentSecretId='single-quoted'
EMPTY=
not a variable line
";
        assert_eq!(dot_env(text, "ANTHROPIC_API_KEY").as_deref(), Some("sk-ant-api03-plain"));
        assert_eq!(dot_env(text, "GEMINI_API_KEY").as_deref(), Some("quoted-and-exported"));
        assert_eq!(dot_env(text, "TencentSecretId").as_deref(), Some("single-quoted"));
        assert_eq!(dot_env(text, "EMPTY").as_deref(), Some(""));
        assert_eq!(dot_env(text, "ABSENT"), None);
        // A comment naming a variable must not be read as setting it.
        assert_eq!(dot_env("# ANTHROPIC_API_KEY=commented-out", "ANTHROPIC_API_KEY"), None);
    }
}
