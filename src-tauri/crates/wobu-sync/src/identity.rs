//! Who this installation is, to every peer it will ever talk to.
//!
//! Before #76 a collaborator was `$USER`, falling back to the literal string
//! `"user"`. Two people on default installs were the same person, and anybody
//! could be anybody by exporting a variable. That is survivable for labelling a
//! conflict file on a share you already trust, and it is not an identity to sync
//! against, so it is gone: a peer is now an ed25519 keypair, and the public half
//! — iroh's `EndpointId` — is the name. Note the type is `EndpointId` and not
//! `NodeId`; that name does not exist in iroh 1.0.
//!
//! ## The secret lives in the keychain, and the reason is the share
//!
//! `wobu/sync` in the OS credential store, beside the BYOK provider keys that
//! `keys.rs` in the shell puts at `wobu/<provider>` — the same `keyring` crate,
//! the same `wobu` service, the same per-installation rule. It is not a second
//! convention and must not become one.
//!
//! It is **never** written into the project folder, and that is not a
//! preference. A project folder is meant to be copied to a NAS, a USB stick and
//! a git repo (`docs/07-file-shares.md`); a key inside one is a key handed to
//! everyone who can mount it, and whoever has it can *be* you to every peer you
//! have ever synced with. `keys.rs` argues this at length for provider
//! credentials, where the loss is somebody else's API bill. Here the loss is
//! your identity, so the rule is the same rule and it is held harder.
//!
//! There is deliberately no file fallback of any kind — not app data, not a
//! dotfile, not the "just for development" `.env` that `keys.rs` allows itself.
//! The moment a *file* can supply this key, the next question is which file, and
//! the answer somebody will reach for is one inside the project.
//!
//! ## An unusable keychain degrades rather than fails
//!
//! Headless Linux, CI, a login keyring nobody has unlocked. [`Identity::load`]
//! has no `Result` and cannot raise: it mints an [`Origin::Ephemeral`] identity
//! and carries on. What that costs is stated plainly rather than hidden — the
//! peer's name changes on every restart, so a collaborator's conflict files stop
//! being attributable to one person — and [`Identity::origin`] is how #83's
//! status UI is expected to say so. An app that refused to open a project
//! because a credential store was locked would be a worse app.
//!
//! ### A store that will not answer *at all* is the same failure
//!
//! That paragraph used to describe only a store that answers with an error, and
//! the gap was load bearing. On Linux a *locked* collection does not refuse a
//! read: `org.freedesktop.Secret.Service.Unlock` hands back a prompt object and
//! the client waits for the `Completed` signal that follows it. Where nothing
//! services that prompt — no `gcr-prompter`, a session the daemon cannot draw
//! on, a keyring daemon wedged mid-unlock — the signal never comes and the
//! D-Bus call never returns. The degradation this module promises could not
//! fire, because nothing had failed yet; the caller was simply parked for ever.
//!
//! So the wait is bounded. `STORE_DEADLINE` is how long the platform gets to
//! produce *something*, and a store still thinking after that is treated as one
//! that would not answer — [`Origin::Ephemeral`], same as a refusal, because
//! from the user's side it is the same event and has the same remedy.
//!
//! ## Nothing here carries the key anywhere
//!
//! No `Serialize`, no `Display`, a hand-written [`Debug`] that prints the alias,
//! and no error type at all — every failure in this module resolves to an
//! `Origin` rather than to a message. That last one is a deliberate difference
//! from `keys.rs`, which does put the platform's own wording in an error
//! `detail`: there, a user pasted a key and has to be told the save failed. Here
//! nobody pasted anything and there is nothing for a user to do, so the one
//! thing a message could add is the risk that a credential store echoes the
//! sixty-four hex characters it just refused into a log file.
//!
//! The secret does not leave this module except into iroh's endpoint builder.
//! It never crosses the Tauri bridge, because there is no shape here that could.

use std::fmt;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use iroh::{EndpointId, SecretKey};

/// The service half of the keychain entry. The same constant `keys.rs` uses, and
/// it has to stay the same: renaming it here would orphan the entry on every
/// machine that already has one and mint a new peer identity for everybody.
const SERVICE: &str = "wobu";

/// The user half, so the identity lives at `wobu/sync`. Sits in the same
/// namespace as `wobu/anthropic` and `wobu/gemini` on purpose — one place a
/// person can look to see everything Wobu has put in their keychain, and one
/// place to delete it all from.
const ENTRY: &str = "sync";

/// How long the OS credential store gets to answer before this process decides
/// it is not going to.
///
/// Ten seconds because that is roughly what a person needs to notice an unlock
/// prompt and type into it — the case worth waiting for — and because a machine
/// where the prompt will never appear should not be indistinguishable from a
/// hung app. The same number as `sync::ONLINE_WAIT` in the shell, for the same
/// reason: wait for the good outcome, then carry on without it rather than for
/// ever.
///
/// It is a ceiling and not a delay. A store that answers in the ordinary two
/// milliseconds is not waited on at all.
const STORE_DEADLINE: Duration = Duration::from_secs(10);

/// Where this process's identity came from.
///
/// Three states rather than a `bool`, because "read back from the keychain" and
/// "minted just now and stored" are both fine and permanent, while the third is
/// neither. Only [`Origin::Ephemeral`] costs the user anything, and it is the
/// one worth surfacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An identity that already existed on this machine. The ordinary case after
    /// the first run.
    Keychain,
    /// There was no entry, so one was generated and written. From here on this
    /// machine reads back the same peer.
    Minted,
    /// The credential store would not answer, or would not accept a write. This
    /// identity works for this process and is gone when it exits, so the peer
    /// name a collaborator sees changes on every restart.
    Ephemeral,
}

/// This installation's peer identity.
///
/// Clone is cheap and safe — iroh's `SecretKey` is itself `Clone` and zeroes on
/// drop — and it is needed because [`crate::Config`] is `Clone` and the app
/// holds one identity across every endpoint it binds.
#[derive(Clone)]
pub struct Identity {
    secret: SecretKey,
    origin: Origin,
}

impl Identity {
    /// The identity this machine syncs as, read from the keychain or created.
    ///
    /// Infallible by construction and bounded by `STORE_DEADLINE`; see the
    /// module documentation for both. Call it once per process and keep the
    /// result — every call reaches the credential store, and on Linux a locked
    /// one prompts the user each time.
    ///
    /// Blocking, still: it is bounded rather than asynchronous, because the
    /// wait belongs to whichever thread the caller is willing to spend and this
    /// crate should not be the one deciding that. The shell calls it off the
    /// main thread for exactly that reason.
    pub fn load() -> Identity {
        Identity::within(STORE_DEADLINE, || Identity::resolve(&OsStore))
    }

    /// Run `load` on its own thread and take the answer, or give up.
    ///
    /// The thread is deliberately abandoned on expiry rather than joined, and
    /// there is no way around that: it is parked inside a platform call with no
    /// timeout and no cancellation, so a join is the hang this function exists
    /// to remove. One leaked thread that is asleep on a socket costs a stack;
    /// what it buys is a window.
    ///
    /// If it does wake up — the user finds the prompt a minute later and unlocks
    /// — it will have minted and stored a key that this run is not using. That
    /// is the good case, not a leak: the *next* launch reads that key back and
    /// gets a stable name, which is the outcome the whole module is for.
    fn within(deadline: Duration, load: impl FnOnce() -> Identity + Send + 'static) -> Identity {
        // Capacity one so the abandoned thread's send never blocks on a receiver
        // that has already gone; it finishes and exits instead of parking again.
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = tx.send(load());
        });
        rx.recv_timeout(deadline).unwrap_or_else(|_| Identity::ephemeral())
    }

    /// A fresh identity that is not stored anywhere.
    ///
    /// For tests, and for any caller that deliberately wants a throwaway peer.
    /// Stable for as long as the value is held, which is what makes it usable in
    /// a test that binds twice and expects the same endpoint id back.
    pub fn ephemeral() -> Identity {
        Identity { secret: SecretKey::generate(), origin: Origin::Ephemeral }
    }

    /// The public key, which is this peer's name and its TLS certificate.
    pub fn id(&self) -> EndpointId {
        self.secret.public()
    }

    /// The short name a person reads: `amber-heron-4f1a`.
    ///
    /// Derived from [`Self::id`] with no table anywhere, which is what makes
    /// `kael-vantris.conflict-amber-heron-4f1a-<ts>.md` mean the same thing on
    /// every machine that sees the file. See [`wobu_core::peer`] for why it is
    /// derived rather than chosen, and for why nothing may authenticate with it.
    pub fn alias(&self) -> String {
        wobu_core::peer::alias(self.id().as_bytes())
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The secret, for the one caller allowed to have it.
    ///
    /// `pub(crate)` rather than `pub`, and that is the boundary: the only thing
    /// in this workspace that can obtain the key is [`crate::Config::bind`],
    /// which hands it to iroh's endpoint builder. Widening this is how a key
    /// starts turning up in a struct that derives `Debug`.
    pub(crate) fn secret_key(&self) -> &SecretKey {
        &self.secret
    }

    /// Keychain, then mint-and-store, then ephemeral — in that order, and the
    /// order is the specification rather than an implementation detail.
    ///
    /// An entry that is present but unreadable is *replaced* rather than treated
    /// as a failure. A truncated or hand-edited value cannot be recovered by
    /// asking again, and refusing to sync until the user finds their own
    /// credential store and deletes a row by hand would be a dead end with no
    /// affordance out of it. The cost is that the peer gets a new name, which is
    /// what happens either way once the old key is unreadable.
    fn resolve(store: &dyn Store) -> Identity {
        if let Ok(Some(stored)) = store.get()
            && let Ok(secret) = stored.trim().parse::<SecretKey>()
        {
            return Identity { secret, origin: Origin::Keychain };
        }

        let secret = SecretKey::generate();
        match store.set(&hex(&secret.to_bytes())) {
            Ok(()) => Identity { secret, origin: Origin::Minted },
            Err(Unavailable) => Identity { secret, origin: Origin::Ephemeral },
        }
    }
}

/// Hand-written, and this is the impl standing between an ed25519 secret key and
/// `{:?}` in a log line. [`crate::Config`] derives `Debug` and holds one of
/// these; so will #82's `SyncManager`. iroh's own `SecretKey` prints
/// `SecretKey(..)`, so this is belt and braces — but the belt is the part that
/// gets replaced by a `#[derive]` one day, and the alias and origin are the two
/// things anybody debugging sync actually wants to see.
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("alias", &self.alias())
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// The store did not answer, or would not take a write. One variant, carrying
/// nothing: the platform's message is not kept, because the platform is under no
/// obligation not to quote the value it just refused.
#[derive(Debug)]
struct Unavailable;

/// The OS credential store.
///
/// A trait for the same reason `keys.rs` has one: CI has no Secret Service and a
/// developer's session may have a locked one, so a test that reached the real
/// keychain would either be skipped everywhere it matters or would write a
/// private key into the machine running it.
trait Store {
    /// `Ok(None)` when there is no entry. The value is whatever was stored, not
    /// necessarily a key — [`Identity::resolve`] decides that.
    fn get(&self) -> Result<Option<String>, Unavailable>;
    fn set(&self, value: &str) -> Result<(), Unavailable>;
}

/// Secret Service on Linux, Keychain on macOS, Credential Manager on Windows —
/// `keyring`'s default `v1` feature, one per platform, exactly as the shell's
/// provider keys use it.
struct OsStore;

impl Store for OsStore {
    fn get(&self) -> Result<Option<String>, Unavailable> {
        match keyring::Entry::new(SERVICE, ENTRY).map_err(|_| Unavailable)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Unavailable),
        }
    }

    fn set(&self, value: &str) -> Result<(), Unavailable> {
        keyring::Entry::new(SERVICE, ENTRY)
            .map_err(|_| Unavailable)?
            .set_password(value)
            .map_err(|_| Unavailable)
    }
}

/// Lowercase hex of the secret's thirty-two bytes.
///
/// The format iroh's own `SecretKey: FromStr` reads back, so the entry is not a
/// private encoding this module would have to keep in step with itself — the
/// parse in [`Identity::resolve`] is iroh's, not ours.
fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Instant;

    use super::*;

    /// A store in memory, or one that refuses everything.
    ///
    /// `RefCell` rather than a lock: no test that reaches a store crosses a
    /// thread — the two that do spawn one drive `within` directly — and a
    /// `Mutex` here would only be ceremony.
    struct FakeStore {
        entry: RefCell<Option<String>>,
        refuses: bool,
    }

    impl FakeStore {
        fn empty() -> FakeStore {
            FakeStore { entry: RefCell::new(None), refuses: false }
        }

        fn holding(value: &str) -> FakeStore {
            FakeStore { entry: RefCell::new(Some(value.to_owned())), refuses: false }
        }

        fn refusing() -> FakeStore {
            FakeStore { entry: RefCell::new(None), refuses: true }
        }

        fn stored(&self) -> Option<String> {
            self.entry.borrow().clone()
        }
    }

    impl Store for FakeStore {
        fn get(&self) -> Result<Option<String>, Unavailable> {
            if self.refuses {
                return Err(Unavailable);
            }
            Ok(self.stored())
        }

        fn set(&self, value: &str) -> Result<(), Unavailable> {
            if self.refuses {
                return Err(Unavailable);
            }
            *self.entry.borrow_mut() = Some(value.to_owned());
            Ok(())
        }
    }

    /* ── resolution ───────────────────────────────────────────────────────── */

    #[test]
    fn a_first_run_mints_an_identity_and_keeps_it() {
        // The property #76 exists for: a peer's name must survive a restart, or
        // it is not a name. Binding with `None` mints a fresh key per bind,
        // which is what this replaces.
        let store = FakeStore::empty();

        let first = Identity::resolve(&store);
        assert_eq!(first.origin(), Origin::Minted);
        assert!(store.stored().is_some(), "the minted key was not written anywhere");

        let second = Identity::resolve(&store);
        assert_eq!(second.origin(), Origin::Keychain);
        assert_eq!(first.id(), second.id(), "the second run came back as a different peer");
        assert_eq!(first.alias(), second.alias());
    }

    #[test]
    fn the_stored_form_is_what_iroh_itself_parses() {
        // Not a private encoding this module would have to keep in step with:
        // sixty-four lowercase hex characters, read back by iroh's own `FromStr`.
        let store = FakeStore::empty();
        let minted = Identity::resolve(&store);

        let stored = store.stored().expect("something was written");
        assert_eq!(stored.len(), 64, "{stored}");
        assert!(stored.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(stored.parse::<SecretKey>().unwrap().public(), minted.id());
    }

    #[test]
    fn an_unreadable_entry_is_replaced_rather_than_being_a_dead_end() {
        // A truncated or hand-edited value cannot be recovered by asking again,
        // and an app that refused to sync until the user opened their own
        // credential store and deleted a row would have no way out of it.
        let store = FakeStore::holding("not a key");

        let identity = Identity::resolve(&store);
        assert_eq!(identity.origin(), Origin::Minted);
        assert_eq!(
            store.stored().unwrap().parse::<SecretKey>().unwrap().public(),
            identity.id(),
            "the junk entry survived"
        );
    }

    #[test]
    fn a_stored_key_with_a_trailing_newline_still_reads() {
        // Not hypothetical: a credential store filled in by hand, or by a
        // deployment script echoing into it, is exactly how this arrives. A
        // rejected key would silently mint a *new* identity and rename the peer.
        let secret = SecretKey::generate();
        let store = FakeStore::holding(&format!("  {}\n", hex(&secret.to_bytes())));

        let identity = Identity::resolve(&store);
        assert_eq!(identity.origin(), Origin::Keychain);
        assert_eq!(identity.id(), secret.public());
    }

    #[test]
    fn an_unusable_credential_store_still_yields_a_working_identity() {
        // Headless Linux, CI, a locked login keyring. None of those may stop the
        // app, and none of them may be reported as a failure — `Origin` is the
        // whole report, and #83's status UI is where the user hears about it.
        let store = FakeStore::refusing();

        let identity = Identity::resolve(&store);
        assert_eq!(identity.origin(), Origin::Ephemeral);
        assert!(!identity.alias().is_empty());
        assert!(store.stored().is_none(), "a refusing store was written to anyway");
    }

    /* ── a store that will not answer ─────────────────────────────────────── */

    #[test]
    fn a_store_that_never_answers_does_not_hold_the_process() {
        // The Linux failure this bound exists for: a locked collection answers a
        // read with a prompt object rather than with an error, and where nothing
        // draws that prompt the D-Bus call never returns. `resolve` is not
        // reached and `Origin::Ephemeral` cannot fire — the caller is simply
        // parked, and on the main thread that is an app with no window.
        let started = Instant::now();
        let identity = Identity::within(Duration::from_millis(50), || {
            thread::sleep(Duration::from_secs(60));
            unreachable!("the deadline should have expired long before this")
        });

        assert_eq!(identity.origin(), Origin::Ephemeral);
        assert!(!identity.alias().is_empty(), "an abandoned wait still names the peer");
        assert!(started.elapsed() < Duration::from_secs(5), "the deadline did not bound the wait");
    }

    #[test]
    fn a_store_that_answers_is_not_waited_on() {
        // The bound is a ceiling and not a delay: the ordinary case is a store
        // that replies in microseconds, and it must not be slowed down or, worse,
        // have its answer discarded in favour of a throwaway identity.
        let secret = SecretKey::generate();
        let expected = secret.public();

        let started = Instant::now();
        let identity = Identity::within(Duration::from_secs(60), move || Identity {
            secret,
            origin: Origin::Keychain,
        });

        assert_eq!(identity.origin(), Origin::Keychain);
        assert_eq!(identity.id(), expected, "the store's answer was thrown away");
        assert!(started.elapsed() < Duration::from_secs(5), "a ready answer was waited on");
    }

    /* ── the key does not leave ───────────────────────────────────────────── */

    #[test]
    fn the_secret_key_never_appears_in_a_debug_dump() {
        // The classic leak: a `#[derive(Debug)]` on the struct that happens to
        // hold the key. `Config` derives `Debug` and holds one of these, and so
        // will #82's manager, so this is the impl standing between an ed25519
        // secret and a log file.
        let identity = Identity::ephemeral();
        let secret = hex(&identity.secret_key().to_bytes());

        let dumped = format!("{identity:?}");
        assert!(!dumped.contains(&secret), "the secret key survived Debug: {dumped}");
        // And a few bytes of it, in case a future impl prints a prefix.
        assert!(!dumped.contains(&secret[..8]), "{dumped}");
        assert!(dumped.contains(&identity.alias()), "the dump is still useful: {dumped}");

        // The same claim about the type one level down, since a `Config` holding
        // an `Identity` is what actually gets printed.
        let config = crate::Config { identity: Some(identity), ..crate::Config::loopback() };
        let dumped = format!("{config:?}");
        assert!(!dumped.contains(&secret), "the secret key crossed `Config`'s Debug: {dumped}");
    }

    /* ── the alias ────────────────────────────────────────────────────────── */

    #[test]
    fn the_alias_ends_with_the_start_of_the_id_as_iroh_prints_it() {
        // `wobu-core` asserts this against raw bytes; this is the half that
        // pins it to a real `EndpointId`, because the claim is about the
        // characters a user compares against a ticket and only iroh decides
        // what those are. iroh 1.0 prints a public key as lowercase hex — the
        // 52-character base32 of iroh 0.x is gone, and so is `NodeId`.
        let identity = Identity::ephemeral();
        let printed = identity.id().to_string();

        assert_eq!(printed.len(), 64, "iroh no longer prints an id as 32 bytes of hex: {printed}");
        assert!(
            identity.alias().ends_with(&printed[..wobu_core::peer::SUFFIX_CHARS]),
            "alias {} does not name id {printed}",
            identity.alias()
        );
    }

    #[test]
    fn an_alias_is_a_filename_before_it_is_anything_else() {
        // It goes into a `.conflict-<peer>-<ts>.md` on a share that may be read
        // from Windows. `wobu-store` parses those names back; a peer name that
        // was not a slug would be a card the app could not label.
        for _ in 0..64 {
            let alias = Identity::ephemeral().alias();
            assert!(wobu_core::is_valid_slug(&alias), "{alias} is not a usable filename");
        }
    }
}
