//! Peer-to-peer transport for a Wobu project: an iroh endpoint, one ALPN, and
//! the one thing two peers must agree on before anything else can happen.
//!
//! This is M3's floor ([#75](https://github.com/krazyjakee/wobu/issues/75)).
//! Nothing here reconciles, transfers, or remembers anything. It gets two
//! machines talking about the same project and hands the connection on.
//!
//! ## We are not writing a handshake
//!
//! This is the thing to get right, and the thing most likely to be got wrong by
//! being helpful. A `wobu/sync/1` connection is **already end-to-end encrypted
//! and mutually authenticated before this crate sees it**. iroh is QUIC with
//! TLS 1.3 where each endpoint's ed25519 public key *is* its TLS certificate, so
//! by the time a `Connection` exists both sides have proved possession of the
//! secret key behind the [`EndpointId`] the other one is looking at.
//! `Connection::remote_id` is that proof's output, not a claim the peer made.
//!
//! So the opening exchange in [`opening`] does exactly one thing: **it states
//! which project this connection is about, and gets back whether the other side
//! holds it.** One ULID out, one word back.
//!
//! What it deliberately does *not* do, and what must not be added here:
//!
//! - No key exchange. There is nothing left to agree; the session keys came out
//!   of the TLS handshake.
//! - No challenge/response, no nonce, no proof-of-possession. Adding one would
//!   re-derive, in application code and worse, the property TLS already
//!   established.
//! - No signature verification. Nothing in this crate reads a `Signature`, and
//!   `iroh::SecretKey` is used only to *be* an identity, never to sign anything.
//! - No authorisation. Holding a project and being *allowed* to sync it are
//!   different questions, and the second one is not answered here. iroh's
//!   `EndpointHooks::after_handshake` is where it would go; it is unused, and
//!   `docs/10-sync-spike.md` flags it as read-but-not-tried.
//!
//! If a change to this crate needs a cryptographic primitive, the change is
//! wrong. That is the whole of the rule.
//!
//! [`ticket::Grant`] is the thing most likely to be read as an exception, so it
//! is worth naming here: it is thirty-two random bytes in a ticket, it is not a
//! key, and nothing derives, signs, encrypts or challenges with it. It exists to
//! tell "I was invited" apart from "I read a project ULID off a shared folder",
//! which is an *authorisation* input and therefore #84's business rather than
//! this crate's — nothing on `wobu/sync/1` presents or checks one today. Asking
//! the OS for random bytes and putting them in a token is not a primitive. Doing
//! anything else with them would be.
//!
//! ## The refusal is the security-relevant path
//!
//! A dialler names a project. If we do not hold it, the connection is refused —
//! and the refusal must say *only* that. A peer who dials with a guessed ULID
//! must learn nothing except whether that guess was right, because "I have never
//! seen that project" and "I have never seen that project, though I hold three
//! others" are different disclosures and only one of them is ours to make.
//!
//! Three things enforce it, at three levels:
//!
//! - [`Projects::holds`] takes one project and returns a bool. An implementation
//!   has no way to hand back a list, so the accept path never has one.
//! - The refusal on the wire is a unit variant with no fields, byte-identical
//!   whatever is held, and the QUIC close code and reason beside it are
//!   constants.
//! - [`Error::ProjectNotHeld`] carries nothing, so a caller cannot log or
//!   surface something the wire did not say.
//!
//! ## Who a peer is
//!
//! [`Identity`] — an ed25519 keypair whose secret half lives in the OS keychain
//! at `wobu/sync`, per installation, never in the project folder. The public
//! half is the [`EndpointId`] above, so a peer's name and its TLS certificate
//! are the same thirty-two bytes, which is why nothing in this crate has to
//! check that they match.
//!
//! Two things follow, and both are easy to get subtly wrong later:
//!
//! - The identity is **not** a `SecretKey` field anybody can read. It is
//!   `pub(crate)` behind [`Identity`], and [`Config::bind`] handing it to iroh's
//!   builder is the only place in the workspace it is touched. The rule against
//!   cryptographic primitives above is what makes that sufficient: the key is
//!   used to *be* somebody, never to sign anything.
//! - [`Identity::alias`] — `amber-heron-4f1a` — is for **display only**. It is
//!   twenty-eight bits, which is a name and not a key, and a peer who wanted to
//!   collide with somebody's alias could grind one out in seconds. Anything
//!   deciding anything compares the full [`EndpointId`].
//!
//! ## Runtime
//!
//! No runtime is created here. iroh wants `tokio ^1.44` and the workspace is on
//! 1.53.1, so the lock unifies and the endpoint runs on the runtime Tauri
//! already started — the same one the job queue uses. A second runtime would put
//! the accept loop on threads the app cannot cancel, which is how a clean quit
//! turns into a hang.
//!
//! ## Lifetime and shutdown
//!
//! [`SyncEndpoint`] holds iroh's `Router`, which is `#[must_use]` and **aborts
//! its accept loop when dropped**. Dropping a `SyncEndpoint` therefore severs
//! every inbound connection at once, silently. Prefer
//! [`SyncEndpoint::shutdown`], which winds the loop down and closes the
//! endpoint, and hold the handle for as long as the project is open. #82's
//! `SyncManager` is the owner this implies.
//!
//! ## What is tested here, and what cannot be
//!
//! [`Reach::Loopback`] binds an endpoint with no relay, no address lookup and
//! one socket on `127.0.0.1`, so two endpoints in one process is a real test of
//! the whole path — ALPN negotiation, the opening exchange, the refusal, the
//! timeout, shutdown. What a single host cannot exercise is equally real and is
//! not implied by any test passing: NAT traversal, holepunching, relay
//! selection, and what happens on a network where the relay itself is blocked.
//! `docs/10-sync-spike.md` records what was observed on two processes against
//! n0's live relays; anything beyond that needs two machines.
//!
//! ## Seams left for the rest of M3
//!
//! - **#76 peer identity** — done; [`identity`] and [`Config::identity`]. What
//!   is *not* done is the wiring: the Tauri shell does not depend on this crate
//!   yet, so nothing calls [`Identity::load`] and nothing hands the alias to
//!   `wobu_store::peer::install`. That belongs to whichever of #77 or #82 first
//!   makes the app hold a `SyncEndpoint`. Until then a conflict sibling is named
//!   from `wobu-store`'s unattributed fallback, which is per *process* rather
//!   than per installation.
//! - **#77 tickets** — done, in [`ticket`]: [`SyncEndpoint::ticket`] mints one
//!   from [`SyncEndpoint::addr`] (await [`SyncEndpoint::online`] first, or the
//!   address has no relay in it and the ticket is undialable off the LAN) and
//!   [`SyncEndpoint::connect_ticket`] accepts one. What is *not* done, and is
//!   the shell's: persisting a ticket in local app data beside the keychain
//!   entry, and cloning a project a [`Disposition::Clone`] ticket names. Neither
//!   belongs here — a transport crate that wrote files would be `identity.rs`'s
//!   argument against a file fallback, lost.
//! - **#79 manifest exchange, #81 blob transfer** — [`Session::connection`] and
//!   [`SyncEndpoint::endpoint`]. The opening exchange finished its stream, so a
//!   later protocol owns whatever streams it opens; blobs registers a second
//!   ALPN on the same router.
//! - **#82 `SyncManager`** — [`Sessions`] is the trait it implements, and the
//!   `Router`'s abort-on-drop is why it must exist.
//! - **#83 status and presence** — [`Session::is_relayed`] now,
//!   `Connection::path_events` when a live badge is wanted.
//! - **#84 authorisation, if it is ever wanted** — not here. See above.

pub mod endpoint;
pub mod error;
pub mod identity;
mod opening;
pub mod ticket;

pub use endpoint::{Config, Projects, Reach, Session, SyncEndpoint, Sessions};
pub use error::{Error, Result};
pub use identity::{Identity, Origin};
pub use ticket::{Disposition, Grant, Ticket};
/// The project identifier, re-exported rather than aliased: the ULID on the wire
/// is the same one the rest of Wobu calls a project, and a second name for it
/// would be a second thing to keep in step.
pub use wobu_core::Id;

/// The ALPN this crate speaks.
///
/// It is the protocol name *and* the version. TLS negotiates it before any
/// application byte is written, so a peer speaking `wobu/sync/2` is refused by
/// iroh before reaching any code here — which is why the opening message carries
/// no version field of its own.
pub const ALPN: &[u8] = b"wobu/sync/1";
