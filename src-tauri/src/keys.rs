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
//! unlocked: none of those are errors and none of them stop the app. Reads and
//! writes fall back to owner-only files under Wobu's application-data directory,
//! never under a project. The UI reports that source but never disables the
//! action; a pasted key succeeds unless both stores are unwritable.
//!
//! "Will not answer" includes a platform call that never returns. Linux Secret
//! Service can wait indefinitely for an unlock prompt nobody can draw, so every
//! operation is run on a disposable thread behind [`STORE_DEADLINE`]. Public
//! operations then await that blocking work away from Tauri's command threads:
//! a wedged keyring can delay an answer, but it cannot freeze the application.
//!
//! See `docs/08-providers.md`.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use serde::Serialize;

use crate::diag;
use crate::error::{Code, CommandResult, WobuError};
use crate::redact;
use wobu_store::paths;

/// The service half of the keychain entry, so a provider's key lives at
/// `wobu/<provider>`. The other half is `TextProvider::id`, which documents the
/// same pairing from the adapter's side — renaming an id orphans every key
/// already stored under the old one, on every machine.
const SERVICE: &str = "wobu";

/// How long the OS credential store gets to answer one operation.
///
/// Linux Secret Service can wait forever for an unlock prompt that no process
/// is able to draw. Provider keys are read from commands the user is waiting
/// on, so an unbounded platform call turns an Enhance click into an apparent
/// application hang. Half a second is enough for a healthy local service and
/// short enough that Settings remains usable before the fallback takes over.
const STORE_DEADLINE: Duration = Duration::from_millis(500);

/* ── the secret ───────────────────────────────────────────────────────────── */

/// A credential, in the one shape that cannot be printed by accident.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// `pub(crate)` so that `enhance.rs`'s tests can build an adapter without a
    /// real credential. Nothing outside this crate can mint one, which is the
    /// property that matters: a `Secret` in the wild has come from one of this
    /// machine's credential stores or the development fallback and nowhere else.
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
    /// Wobu's owner-only fallback under the application-data directory. Used
    /// only when the operating-system store cannot answer.
    Local,
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
    /// failure: Wobu's private local fallback remains writable and the UI keeps
    /// offering the field.
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
    /// A property of the native OS store, reported per provider so Settings can
    /// explain why a key is using Wobu's private local fallback.
    pub keychain: Keychain,
}

/// What a delete did, and what the provider resolves to afterwards.
///
/// A shape rather than a bare `bool` because two outcomes are easy to confuse
/// and both are ordinary. "There was nothing stored" is not a failure. And on a
/// developer's machine, removing a stored entry can leave the provider
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

/// Durable fallback when the operating-system credential service cannot be
/// used. Each value is a separate owner-only file under Wobu's application
/// data directory, never under an open project.
struct LocalStore {
    root: PathBuf,
    io: Mutex<()>,
}

impl LocalStore {
    fn new(root: PathBuf) -> LocalStore {
        LocalStore { root, io: Mutex::new(()) }
    }

    fn path(&self, provider: &str, suffix: &str) -> Result<PathBuf, Unavailable> {
        if provider.is_empty() || !provider.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(Unavailable("invalid provider credential id".into()));
        }
        Ok(self.root.join(format!("{provider}{suffix}")))
    }

    fn prepare(&self) -> Result<(), Unavailable> {
        std::fs::create_dir_all(&self.root).map_err(local_error)?;
        restrict_dir(&self.root).map_err(local_error)
    }

    fn write(&self, path: &Path, value: &[u8]) -> Result<(), Unavailable> {
        self.prepare()?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(local_error)?;
        file.write_all(value).map_err(local_error)?;
        file.sync_all().map_err(local_error)?;
        paths::restrict(path).map_err(local_error)
    }

    fn ignores_keychain(&self, provider: &str) -> Result<bool, Unavailable> {
        let _guard = self.io.lock();
        Ok(self.path(provider, ".ignore-keychain")?.is_file())
    }

    fn ignore_keychain(&self, provider: &str) -> Result<(), Unavailable> {
        let _guard = self.io.lock();
        let path = self.path(provider, ".ignore-keychain")?;
        self.write(&path, b"")
    }

    fn trust_keychain(&self, provider: &str) -> Result<(), Unavailable> {
        let _guard = self.io.lock();
        remove_if_present(&self.path(provider, ".ignore-keychain")?).map_err(local_error)?;
        Ok(())
    }
}

impl Store for LocalStore {
    fn get(&self, provider: &str) -> Result<Option<Secret>, Unavailable> {
        let _guard = self.io.lock();
        let path = self.path(provider, ".key")?;
        match std::fs::read_to_string(path) {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(local_error(error)),
        }
    }

    fn set(&self, provider: &str, secret: &str) -> Result<(), Unavailable> {
        let _guard = self.io.lock();
        let path = self.path(provider, ".key")?;
        self.write(&path, secret.as_bytes())
    }

    fn delete(&self, provider: &str) -> Result<bool, Unavailable> {
        let _guard = self.io.lock();
        remove_if_present(&self.path(provider, ".key")?).map_err(local_error)
    }
}

fn local_error(error: std::io::Error) -> Unavailable {
    Unavailable(format!("private local credential store: {error}"))
}

fn remove_if_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn restrict_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
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

struct StoreAccess {
    active_since: Mutex<Option<Instant>>,
    idle: Condvar,
}

impl Default for StoreAccess {
    fn default() -> Self {
        Self { active_since: Mutex::new(None), idle: Condvar::new() }
    }
}

/// How a development-time variable is read.
type ReadVar = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The process-wide answer to "what is this provider's key".
///
/// Managed Tauri state, registered in `lib.rs`. Holds no reference to the open
/// project and never will: a key belongs to the installation, and a type that
/// could see a project folder is a type that could one day read a key out of
/// one.
#[derive(Clone)]
pub struct Keys {
    store: Arc<dyn Store>,
    local: Arc<LocalStore>,
    /// One platform call at a time. A timed-out Secret Service read may still
    /// be waiting for an unlock prompt because the platform API offers no
    /// cancellation; this gate stops every click made behind it from spawning
    /// another permanently parked thread.
    access: Arc<StoreAccess>,
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
    cache: Arc<Mutex<HashMap<String, Lookup>>>,
    store_deadline: Duration,
}

impl Default for Keys {
    fn default() -> Keys {
        Keys {
            store: Arc::new(OsStore),
            local: Arc::new(LocalStore::new(paths::app_data_dir().join("credentials"))),
            access: Arc::default(),
            var: Arc::new(dev_var),
            cache: Arc::default(),
            store_deadline: STORE_DEADLINE,
        }
    }
}

impl Keys {
    /// The key for a provider, or `None` when this machine has none.
    ///
    /// The only way a [`Secret`] leaves this module, and it goes to an adapter
    /// constructor — `AnthropicProvider::new(key)` — rather than into a struct
    /// the UI can see.
    pub async fn secret(&self, provider: &str) -> CommandResult<Option<Secret>> {
        let keys = self.clone();
        let provider = provider.to_owned();
        tauri::async_runtime::spawn_blocking(move || keys.lookup(&provider).secret)
            .await
            .map_err(task_lost)
    }

    /// Several credentials resolved as one user action.
    ///
    /// Tencent signs with a SecretId/SecretKey pair. If the first lookup proves
    /// the store unavailable, the rest use their environment fallbacks without
    /// asking the same locked store again, so one click has one deadline.
    pub async fn secrets(&self, providers: Vec<String>) -> CommandResult<Vec<Option<Secret>>> {
        let keys = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            keys.lookups_blocking(providers).into_iter().map(|found| found.secret).collect()
        })
        .await
        .map_err(task_lost)
    }

    /// Everything the UI is allowed to know.
    pub async fn statuses(&self, providers: Vec<String>) -> CommandResult<Vec<KeyStatus>> {
        let keys = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            providers
                .iter()
                .zip(keys.lookups_blocking(providers.clone()))
                .map(|(provider, found)| Self::status_of(provider, &found))
                .collect()
        })
        .await
        .map_err(task_lost)
    }

    /// Store a key for a provider, replacing any entry already there.
    pub async fn set(&self, provider: String, key: String) -> CommandResult<KeyStatus> {
        let keys = self.clone();
        tauri::async_runtime::spawn_blocking(move || keys.set_blocking(&provider, &key))
            .await
            .map_err(task_lost)?
    }

    fn set_blocking(&self, provider: &str, key: &str) -> CommandResult<KeyStatus> {
        // A pasted key arrives with a trailing newline more often than not, and
        // a key with one on the end is a 401 that reads exactly like a wrong
        // key — which sends the user back to the console for a replacement that
        // will fail the same way.
        let key = key.trim();
        if key.is_empty() {
            return Err(WobuError::new(Code::Invalid, "A key cannot be empty."));
        }

        let store = Arc::clone(&self.store);
        let provider_owned = provider.to_owned();
        let key_owned = key.to_owned();
        let found = match self.within(move || store.set(&provider_owned, &key_owned)) {
            Ok(()) => {
                // Prefer the native store whenever it answers. A successful
                // replacement also retires any fallback and its tombstone.
                if let Err(error) = self.local.delete(provider) {
                    diag::info(format!(
                        "could not retire local credential for {provider}: {}",
                        error.0
                    ));
                }
                if let Err(error) = self.local.trust_keychain(provider) {
                    diag::info(format!(
                        "could not retire credential tombstone for {provider}: {}",
                        error.0
                    ));
                }
                Lookup {
                    source: Some(Source::Keychain),
                    secret: Some(Secret::new(key)),
                    keychain: Keychain::Ready,
                }
            }
            Err(error) => {
                // The user's action still succeeds. This local value wins over
                // any native write that completes after its timeout; Remove
                // writes the tombstone that keeps such a value from returning.
                diag::info(format!(
                    "using private local credential store for {provider}: {}",
                    error.0
                ));
                self.local.set(provider, key).map_err(local_save_failed)?;
                Lookup {
                    source: Some(Source::Local),
                    secret: Some(Secret::new(key)),
                    keychain: Keychain::Unavailable,
                }
            }
        };
        self.cache.lock().insert(provider.to_owned(), found.clone());
        Ok(Self::status_of(provider, &found))
    }

    /// Remove this machine's stored key for a provider.
    pub async fn delete(&self, provider: String) -> CommandResult<KeyRemoval> {
        let keys = self.clone();
        tauri::async_runtime::spawn_blocking(move || keys.delete_blocking(&provider))
            .await
            .map_err(task_lost)?
    }

    fn delete_blocking(&self, provider: &str) -> CommandResult<KeyRemoval> {
        let cached = self.cache.lock().get(provider).cloned();
        let local_removed = self.local.delete(provider).map_err(local_remove_failed)?;
        let store = Arc::clone(&self.store);
        let provider_owned = provider.to_owned();
        let (keychain, native_removed) = match self.within(move || store.delete(&provider_owned)) {
            Ok(removed) => {
                self.local.trust_keychain(provider).map_err(local_remove_failed)?;
                (Keychain::Ready, removed)
            }
            Err(error) => {
                diag::info(format!(
                    "could not remove native credential for {provider}: {}",
                    error.0
                ));
                // Make deletion effective for Wobu even if the OS service is
                // unavailable or completes an earlier write after its timeout.
                self.local.ignore_keychain(provider).map_err(local_remove_failed)?;
                (Keychain::Unavailable, false)
            }
        };
        let removed = local_removed
            || native_removed
            || cached.is_some_and(|found| {
                matches!(found.source, Some(Source::Keychain | Source::Local))
            });
        self.forget(provider);
        // A successful delete definitively means there is no stored value. Use
        // the development fallback directly instead of immediately asking the
        // same credential store to prove the deletion happened.
        let found = match self.env_secret(provider) {
            Some(secret) => {
                Lookup { source: Some(Source::Environment), secret: Some(secret), keychain }
            }
            None => Lookup::unconfigured(keychain),
        };
        self.cache.lock().insert(provider.to_owned(), found.clone());
        Ok(KeyRemoval { removed, status: Self::status_of(provider, &found) })
    }

    /// Existing local fallback, then keychain, environment, unconfigured. A
    /// fallback wins after creation so Enhance never retries a native service
    /// already known to hang.
    fn lookup(&self, provider: &str) -> Lookup {
        let cached = self.cache.lock().get(provider).cloned();
        if let Some(hit) = cached {
            return hit;
        }

        // A local fallback is deliberately first. It exists only because an OS
        // call already failed, so retrying that call on every Enhance would
        // put the stall straight back into the user's workflow.
        match self.local.get(provider) {
            Ok(Some(secret)) => {
                let found = Lookup {
                    source: Some(Source::Local),
                    secret: Some(secret),
                    keychain: Keychain::Unavailable,
                };
                self.cache.lock().insert(provider.to_owned(), found.clone());
                return found;
            }
            Ok(None) => {}
            Err(error) => {
                diag::info(format!(
                    "could not read private local credential for {provider}: {}",
                    error.0
                ));
            }
        }

        if self.local.ignores_keychain(provider).unwrap_or(false) {
            let found = self.cached_or_fallback(provider, Keychain::Unavailable);
            self.cache.lock().insert(provider.to_owned(), found.clone());
            return found;
        }

        let store = Arc::clone(&self.store);
        let provider_owned = provider.to_owned();
        let (keychain, stored) = match self.within(move || store.get(&provider_owned)) {
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

    fn lookups_blocking(&self, providers: Vec<String>) -> Vec<Lookup> {
        let mut store_unavailable = false;
        providers
            .into_iter()
            .map(|provider| {
                let found = if store_unavailable {
                    self.cached_or_fallback(&provider, Keychain::Unavailable)
                } else {
                    self.lookup(&provider)
                };
                store_unavailable |= found.keychain == Keychain::Unavailable;
                found
            })
            .collect()
    }

    #[cfg(test)]
    fn status_blocking(&self, provider: &str) -> KeyStatus {
        let found = self.lookup(provider);
        Self::status_of(provider, &found)
    }

    /// Resolve without touching the OS store, used for the remaining rows of a
    /// batched status request after the first row proves the store unavailable.
    fn cached_or_fallback(&self, provider: &str, keychain: Keychain) -> Lookup {
        if let Some(found) = self.cache.lock().get(provider).cloned() {
            return found;
        }
        if let Ok(Some(secret)) = self.local.get(provider) {
            return Lookup { source: Some(Source::Local), secret: Some(secret), keychain };
        }
        match self.env_secret(provider) {
            Some(secret) => {
                Lookup { source: Some(Source::Environment), secret: Some(secret), keychain }
            }
            None => Lookup::unconfigured(keychain),
        }
    }

    fn status_of(provider: &str, found: &Lookup) -> KeyStatus {
        KeyStatus { provider: provider.to_owned(), source: found.source, keychain: found.keychain }
    }

    /// Run one platform operation on a disposable thread and stop waiting for
    /// it after the credential-store deadline.
    ///
    /// The platform API has no cancellation. On timeout the thread is detached;
    /// if an unlock prompt is eventually answered it exits normally, while the
    /// command that started it has already degraded to an unavailable store.
    fn within<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<T, Unavailable> + Send + 'static,
    ) -> Result<T, Unavailable> {
        self.claim_store()?;

        let (tx, rx) = mpsc::sync_channel(1);
        let access = Arc::clone(&self.access);
        thread::spawn(move || {
            // A store implementation should not panic, but leaving `active`
            // latched forever if one does would turn one platform bug into a
            // permanent process-wide outage.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                .unwrap_or_else(|_| Err(Unavailable("credential store operation panicked".into())));
            *access.active_since.lock() = None;
            access.idle.notify_all();
            let _ = tx.send(result);
        });
        rx.recv_timeout(self.store_deadline).unwrap_or_else(|_| {
            Err(Unavailable(format!(
                "credential store did not answer within {} milliseconds",
                self.store_deadline.as_millis()
            )))
        })
    }

    /// Claim the single platform-call slot, sharing the original deadline with
    /// callers that arrive while a healthy operation is still finishing.
    fn claim_store(&self) -> Result<(), Unavailable> {
        let mut active = self.access.active_since.lock();
        loop {
            let Some(started) = *active else {
                *active = Some(Instant::now());
                return Ok(());
            };
            let Some(remaining) = self.store_deadline.checked_sub(started.elapsed()) else {
                return Err(Unavailable(
                    "credential store is still waiting for an earlier operation".into(),
                ));
            };
            if self.access.idle.wait_for(&mut active, remaining).timed_out() && active.is_some() {
                return Err(Unavailable(
                    "credential store is still waiting for an earlier operation".into(),
                ));
            }
        }
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

fn local_save_failed(error: Unavailable) -> WobuError {
    WobuError::new(
        Code::ProviderKeychainUnavailable,
        "The key could not be saved to either this computer's credential store or Wobu's private local store.",
    )
    .with_detail(error.0)
}

fn local_remove_failed(error: Unavailable) -> WobuError {
    WobuError::new(
        Code::ProviderKeychainUnavailable,
        "The stored key could not be removed from this computer.",
    )
    .with_detail(error.0)
}

fn task_lost(error: impl std::fmt::Display) -> WobuError {
    WobuError::new(Code::Internal, "The credential-store task stopped unexpectedly.")
        .with_detail(error.to_string())
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
/// A shipped Wobu has one fixed owner-only fallback directory under application
/// data. This development exception reads a human-edited file, so its path is
/// compile-time fixed and can never be redirected into a project on a share.
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A store that lives in memory, or refuses to answer at all.
    ///
    /// `fails_with` carries a made-up platform message; one test loads it with a
    /// key, because a platform error that echoes the failing request is exactly
    /// how a credential ends up in `detail`.
    #[derive(Default)]
    struct FakeStore {
        entries: Mutex<HashMap<String, String>>,
        fails_with: Option<String>,
        delay: Duration,
        calls: Arc<AtomicUsize>,
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

        fn stalling(delay: Duration) -> FakeStore {
            FakeStore { delay, ..FakeStore::default() }
        }

        fn refuse(&self) -> Option<Unavailable> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
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
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "wobu-local-credentials-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        keys_at(store, env, root)
    }

    fn keys_at(store: FakeStore, env: &[(&str, &str)], local_root: PathBuf) -> Keys {
        let env: HashMap<String, String> =
            env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Keys {
            store: Arc::new(store),
            local: Arc::new(LocalStore::new(local_root)),
            access: Arc::default(),
            var: Arc::new(move |name| env.get(name).cloned()),
            cache: Arc::default(),
            store_deadline: STORE_DEADLINE,
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

        // 3. A native-store failure takes the private local route, whose status
        //    is still safe to send across the bridge.
        let refusing = keys(FakeStore::refusing(&format!("store rejected api_key={KEY}")), &[]);
        let fallback = refusing.set_blocking("anthropic", KEY).unwrap();
        let serialised = json(&fallback).to_string();
        assert!(!serialised.contains(KEY), "a key reached the webview in a status: {serialised}");

        // 4. Everything this module hands the UI, whatever a future field on it
        //    might hold.
        let configured = keys(FakeStore::holding("anthropic", KEY), &[]);
        let status = json(&configured.status_blocking("anthropic")).to_string();
        assert!(!status.contains(KEY), "a key reached the webview in a status: {status}");
        let removal = json(&configured.delete_blocking("anthropic").unwrap()).to_string();
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
        let status = json(&configured.status_blocking("anthropic"));

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
        assert_eq!(both.status_blocking("anthropic").source, Some(Source::Keychain));
        assert_eq!(both.lookup("anthropic").secret.unwrap().expose(), "from-keychain");

        let env_only = keys(FakeStore::default(), &[("ANTHROPIC_API_KEY", "from-env")]);
        assert_eq!(env_only.status_blocking("anthropic").source, Some(Source::Environment));
        assert_eq!(env_only.lookup("anthropic").secret.unwrap().expose(), "from-env");

        let neither = keys(FakeStore::default(), &[]);
        assert_eq!(neither.status_blocking("gemini").source, None);
        assert!(neither.lookup("gemini").secret.is_none());
    }

    #[test]
    fn an_absent_keychain_is_unconfigured_rather_than_an_error() {
        // Headless Linux, a CI box, a session whose keyring is locked. Reading a
        // key has no `Result` at all, so there is no path from here to a dialog.
        let headless = keys(FakeStore::refusing("no Secret Service on the session bus"), &[]);

        let status = headless.status_blocking("gemini");
        assert_eq!(status.keychain, Keychain::Unavailable);
        assert_eq!(status.source, None);
        assert!(headless.lookup("gemini").secret.is_none());
    }

    #[test]
    fn a_keychain_that_never_answers_is_bounded_for_reads_writes_and_deletes() {
        // Linux Secret Service can wait forever for a prompt's `Completed`
        // signal. Use a store much slower than this test's deadline to prove
        // that no provider operation waits for the platform call to return.
        let store = FakeStore::stalling(Duration::from_millis(100));
        let calls = Arc::clone(&store.calls);
        let mut keys = keys(store, &[]);
        keys.store_deadline = Duration::from_millis(10);

        let started = std::time::Instant::now();
        let status = keys.status_blocking("anthropic");
        assert_eq!(status.keychain, Keychain::Unavailable);
        assert!(started.elapsed() < Duration::from_millis(80));

        let started = std::time::Instant::now();
        let set = keys.set_blocking("anthropic", "sk-ant-api03-real").unwrap();
        assert_eq!(set.source, Some(Source::Local));
        assert_eq!(set.keychain, Keychain::Unavailable);
        assert!(started.elapsed() < Duration::from_millis(80));

        let started = std::time::Instant::now();
        let delete = keys.delete_blocking("anthropic").unwrap();
        assert!(delete.removed);
        assert_eq!(delete.status.source, None);
        assert!(started.elapsed() < Duration::from_millis(80));

        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "the clicks behind the timed-out read must not spawn more blocked platform calls"
        );
    }

    #[test]
    fn concurrent_healthy_lookups_wait_for_the_shared_store_slot() {
        // Settings and the status bar can refresh together. A healthy lookup
        // already in progress must serialize the second one instead of making
        // the second surface falsely report that the keychain is unavailable.
        let store = FakeStore::stalling(Duration::from_millis(20));
        let calls = Arc::clone(&store.calls);
        let mut keys = keys(store, &[]);
        keys.store_deadline = Duration::from_millis(100);

        let first_keys = keys.clone();
        let first = thread::spawn(move || first_keys.status_blocking("anthropic"));
        while calls.load(Ordering::Relaxed) == 0 {
            thread::yield_now();
        }

        let second = keys.status_blocking("gemini");
        assert_eq!(first.join().unwrap().keychain, Keychain::Ready);
        assert_eq!(second.keychain, Keychain::Ready);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_multi_key_lookup_stops_asking_after_the_store_is_unavailable() {
        let store = FakeStore::refusing("locked");
        let calls = Arc::clone(&store.calls);
        let keys = keys(
            store,
            &[("TencentSecretId", "id-from-env"), ("TencentSecretKey", "key-from-env")],
        );

        let found =
            keys.lookups_blocking(vec!["tencent-secret-id".into(), "tencent-secret-key".into()]);
        assert_eq!(found[0].secret.as_ref().map(Secret::expose), Some("id-from-env"));
        assert_eq!(found[1].secret.as_ref().map(Secret::expose), Some("key-from-env"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_developer_fallback_still_works_with_no_keychain_at_all() {
        // The CI shape: no credential store, a key in the environment. It has to
        // resolve, or every provider test on a build box is unconfigured.
        let ci = keys(
            FakeStore::refusing("no Secret Service on the session bus"),
            &[("ANTHROPIC_API_KEY", "from-env")],
        );
        let status = ci.status_blocking("anthropic");
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
        assert_eq!(keys.status_blocking("anthropic").keychain, Keychain::Unavailable);

        keys.store = Arc::new(FakeStore::holding("anthropic", "unlocked-now"));
        assert_eq!(keys.status_blocking("anthropic").keychain, Keychain::Ready);
        assert_eq!(keys.status_blocking("anthropic").source, Some(Source::Keychain));
    }

    /* ── writing and removing ─────────────────────────────────────────────── */

    #[test]
    fn deleting_a_key_is_told_apart_from_there_never_having_been_one() {
        let keys = keys(FakeStore::holding("anthropic", "sk-ant-api03-real"), &[]);

        let first = keys.delete_blocking("anthropic").unwrap();
        assert!(first.removed, "an entry existed and should report as removed");
        assert_eq!(first.status.source, None);

        let second = keys.delete_blocking("anthropic").unwrap();
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

        let removal = keys.delete_blocking("anthropic").unwrap();
        assert!(removal.removed);
        assert_eq!(removal.status.source, Some(Source::Environment));
    }

    #[test]
    fn a_stored_key_replaces_whatever_was_there_and_is_visible_at_once() {
        // The cache is the thing being guarded: a save that the next read does
        // not see would show the user their old state and read as a failed save.
        let keys = keys(FakeStore::default(), &[]);
        assert_eq!(keys.status_blocking("anthropic").source, None);

        let status = keys.set_blocking("anthropic", "sk-ant-api03-first").unwrap();
        assert_eq!(status.source, Some(Source::Keychain));
        assert_eq!(keys.lookup("anthropic").secret.unwrap().expose(), "sk-ant-api03-first");

        keys.set_blocking("anthropic", "sk-ant-api03-second").unwrap();
        assert_eq!(keys.lookup("anthropic").secret.unwrap().expose(), "sk-ant-api03-second");
    }

    #[test]
    fn a_pasted_key_loses_the_whitespace_it_was_pasted_with() {
        // A key with a trailing newline is a 401 that reads exactly like a wrong
        // key, which sends the user back to the console for a replacement that
        // fails identically.
        let keys = keys(FakeStore::default(), &[]);
        keys.set_blocking("anthropic", "  sk-ant-api03-pasted\n").unwrap();
        assert_eq!(keys.lookup("anthropic").secret.unwrap().expose(), "sk-ant-api03-pasted");
    }

    #[test]
    fn an_empty_key_is_refused_rather_than_stored() {
        // Storing one would report the provider as configured and then fail
        // every request, which is the worst of both states.
        let keys = keys(FakeStore::default(), &[]);
        let e = keys.set_blocking("anthropic", "   \n").expect_err("an empty key is not a key");
        assert_eq!(json(&e)["code"], "node.invalid");
        assert_eq!(keys.status_blocking("anthropic").source, None);
    }

    #[test]
    fn an_empty_environment_variable_is_not_a_key() {
        // `.env.example` ships every variable empty, and a developer who copies
        // it without filling anything in must read as unconfigured rather than
        // as configured-and-broken.
        let keys = keys(FakeStore::default(), &[("ANTHROPIC_API_KEY", "  ")]);
        assert_eq!(keys.status_blocking("anthropic").source, None);
    }

    #[test]
    fn a_refused_keychain_saves_to_the_private_local_store_and_survives_restart() {
        let root = std::env::temp_dir()
            .join(format!("wobu-local-credentials-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let keys = keys_at(FakeStore::refusing("the collection is locked"), &[], root.clone());

        let status = keys.set_blocking("gemini", "gemini-token-real").unwrap();
        assert_eq!(status.source, Some(Source::Local));
        assert_eq!(status.keychain, Keychain::Unavailable);

        let store = FakeStore::refusing("still locked");
        let native_calls = Arc::clone(&store.calls);
        let restarted = keys_at(store, &[], root.clone());
        let found = restarted.lookup("gemini");
        assert_eq!(found.source, Some(Source::Local));
        assert_eq!(found.secret.unwrap().expose(), "gemini-token-real");
        assert_eq!(native_calls.load(Ordering::Relaxed), 0);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.join("gemini.key")).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removing_a_fallback_keeps_a_late_native_value_from_reappearing() {
        let root = std::env::temp_dir()
            .join(format!("wobu-local-credentials-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let keys = keys_at(FakeStore::refusing("locked"), &[], root.clone());
        keys.set_blocking("gemini", "new-local-value").unwrap();
        assert!(keys.delete_blocking("gemini").unwrap().removed);

        // Models a native write that completed after Wobu's timeout. The
        // delete tombstone is authoritative until a later explicit Save can
        // reach the native store and clear it.
        let restarted =
            keys_at(FakeStore::holding("gemini", "late-or-stale-native-value"), &[], root.clone());
        assert!(restarted.lookup("gemini").secret.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saving_reports_failure_only_when_both_stores_are_unwritable() {
        let root = std::env::temp_dir()
            .join(format!("wobu-local-credentials-blocked-{}", std::process::id()));
        let _ = std::fs::remove_file(&root);
        std::fs::write(&root, "not a directory").unwrap();
        let keys = keys_at(FakeStore::refusing("locked"), &[], root.clone());

        let error = keys.set_blocking("gemini", "never-serialised").unwrap_err();
        assert_eq!(json(&error)["code"], "provider.keychain_unavailable");
        assert!(!json(&error).to_string().contains("never-serialised"));
        let _ = std::fs::remove_file(root);
    }

    #[test]
    fn a_provider_id_cannot_escape_the_private_credential_directory() {
        let root = std::env::temp_dir()
            .join(format!("wobu-local-credentials-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let local = LocalStore::new(root.clone());
        assert!(local.set("../outside", "secret").is_err());
        assert!(!root.parent().unwrap().join("outside.key").exists());
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
