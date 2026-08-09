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
//! secret key behind the `EndpointId` the other one is looking at.
//! `Connection::remote_id` is that proof's output, not a claim the peer made.
//!
//! So the opening exchange in `opening` does exactly one thing: **it states
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
//! - No authorisation field in the opening message. #90 carries the grant on a
//!   second stream and gives it to [`Projects::admits`] beside the project id.
//!   `EndpointHooks::after_handshake` cannot do that: it runs before any
//!   application byte, so it has no project id, and its rejection would carry
//!   iroh's own close code instead of the byte-identical `ProjectNotHeld`
//!   refusal required here.
//!
//! If a change to this crate needs a cryptographic primitive, the change is
//! wrong. That is the whole of the rule.
//!
//! [`blobs`] is where that rule did the most work, so it is worth reading as the
//! case study rather than as an exception. Moving file content means finding out
//! whether the bytes that arrived are the bytes that were asked for, which is a
//! hash — and the rule said no. So the transfer is `iroh-blobs`, which verifies
//! the BLAKE3 tree chunk by chunk as it arrives, and nothing in this crate
//! computes, compares or stores a digest. What would have been two hundred lines
//! of hand-rolled framing and one `blake3::hash` call is a dependency and no
//! primitive. The rule picked the better design; that is generally what it is
//! for.
//!
//! [`ticket::Grant`] is the thing most likely to be read as an exception, so it
//! is worth naming here: it is thirty-two random bytes in a ticket, it is not a
//! key, and nothing derives, signs, encrypts or challenges with it. It exists to
//! tell "I was invited" apart from "I read a project ULID off a shared folder",
//! which is an *authorisation* input. `wobu/sync/1` presents it on a second QUIC
//! stream, leaving the established opening message untouched. Asking the OS for
//! random bytes, transporting them, and comparing them in constant time adds no
//! key exchange, encryption, signature, or challenge-response.
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
//! - [`Projects::admits`] takes one project and one optional grant and returns a
//!   bool. An implementation has no way to hand back a list or a refusal cause,
//!   so the accept path never has one.
//! - The refusal on the wire is a unit variant with no fields, byte-identical
//!   whatever is held, and the QUIC close code and reason beside it are
//!   constants.
//! - [`Error::ProjectNotHeld`] carries nothing, so a caller cannot log or
//!   surface something the wire did not say.
//!
//! ## A peer not having something is not a peer having deleted it
//!
//! The second standing rule of this crate, and the one with teeth. [`manifest`]
//! swaps a full list of what each side holds, and a node in our list and not in
//! theirs is ambiguous between "they deleted it" and "they never had it". M3 has
//! no tombstones, so nothing on the wire can tell those apart, and **every
//! absence is read as the second one** — by this crate, and by
//! `wobu_store::apply::decide` one crate over, which reaches the same conclusion
//! from the same reasoning.
//!
//! So a delete does not propagate. Remove a node on one machine and it is still
//! on the other; sync again and it may come back. That is a documented
//! shortcoming rather than a bug, because the alternative is not close: an absent
//! entry is also what a half-mounted share, a failed transfer and a manifest this
//! crate truncated all look like, and a delete driven by absence turns any one of
//! those into a world-wide erase of somebody else's writing. A delete that
//! quietly comes back is an annoyance; a delete that quietly happens is
//! unrecoverable.
//!
//! Making deletion work is a schema change — a replicated record saying a
//! deletion happened, with rules about when it may be forgotten — and not a wire
//! change. Nothing may be added to [`manifest`] to approximate one.
//!
//! ## Who a peer is
//!
//! [`Identity`] — an ed25519 keypair whose secret half lives in the OS keychain
//! at `wobu/sync`, per installation, never in the project folder. The public
//! half is the `EndpointId` above, so a peer's name and its TLS certificate
//! are the same thirty-two bytes, which is why nothing in this crate has to
//! check that they match.
//!
//! Two things follow, and both are easy to get subtly wrong later:
//!
//! - The identity is **not** a `SecretKey` field anybody can read. It is
//!   `pub(crate)` behind [`Identity`], and `Config::bind` handing it to iroh's
//!   builder is the only place in the workspace it is touched. The rule against
//!   cryptographic primitives above is what makes that sufficient: the key is
//!   used to *be* somebody, never to sign anything.
//! - [`Identity::alias`] — `amber-heron-4f1a` — is for **display only**. It is
//!   twenty-eight bits, which is a name and not a key, and a peer who wanted to
//!   collide with somebody's alias could grind one out in seconds. Anything
//!   deciding anything compares the full `EndpointId`.
//!
//! ## Runtime
//!
//! No runtime is created here. iroh wants `tokio ^1.44` and the workspace is on
//! 1.53.1, so the lock unifies and the endpoint runs on the runtime Tauri
//! already started — the same one the job queue uses. A second runtime would put
//! the accept loop on threads the app cannot cancel, which is how a clean quit
//! turns into a hang.
//!
//! **One qualification, and it is `iroh-blobs`' rather than ours.**
//! `iroh_blobs::store::fs::FsStore::load` builds its own multi-threaded runtime
//! for the store's actor, so a process holding a [`Blobs`] has threads this
//! crate did not start and cannot see. That is not a licence to start more: no
//! code here creates a runtime, and the paragraph above is the rule for anything
//! added. It is a reason [`Blobs::shutdown`] is not optional housekeeping —
//! it is the only thing that stops them, and the hang it prevents is exactly the
//! one the rule was written about.
//!
//! ## Filesystem
//!
//! [`blobs`] touches one, and it is the only module that does. Everything else —
//! the opening exchange, [`manifest`], [`ticket`] — moves lists and hands them back, and
//! `identity.rs` argues at length that a transport crate which wrote files would
//! be the wrong shape. That argument still stands for keys and for content, and
//! #81 is not a counterexample to it: the bytes have to land somewhere, the only
//! code that sees them is here, and the alternative is buffering a four-gigabyte
//! asset in memory to hand to a caller that would write it to the same path.
//!
//! What follows from that concession is a boundary rather than a licence.
//! [`Blobs`] is given a project root and writes underneath it; it creates
//! nothing else, deletes nothing ever, and reads nothing it was not handed a
//! path to. [`blobs::place`] is the one function in the workspace where a string
//! a stranger wrote becomes a real path, and it is written as if nothing had
//! checked it before.
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
//! - **#79 manifest exchange** — done, in [`manifest`]: [`manifest::exchange`]
//!   swaps `(node id, hash)` for every node and `(rel_path, hash)` for every blob
//!   under `assets/` and `generations/`, paged and capped, in both directions at
//!   once. It takes and returns the exact `Vec<(Id, String)>` that
//!   `wobu_store::Project::manifest` produces and `plan_against_peer` consumes,
//!   so the shell adapts nothing. **This crate does not diff manifests** — #80
//!   landed that as a pure function in `wobu_store::apply::decide`, and a second
//!   copy of it here would be the milestone's most consequential decision made
//!   twice, in the crate with no filesystem to test it against.
//!   What is *not* done is the wiring, for the same reason as #76: the shell does
//!   not depend on this crate yet, so nothing assembles the blob half or feeds
//!   the node half back into a `Project`.
//! - **#81 blob transfer** — done, in [`blobs`]: [`Config::blobs`] hands a
//!   [`Blobs`] to [`SyncEndpoint::bind`], which registers `iroh-blobs`' ALPN as a
//!   second protocol on the same router, the same key and the same socket.
//!   [`Blobs::offer`] makes a list servable, [`Blobs::fetch`] pulls what is
//!   missing from one peer over a connection of its own, and every path is put
//!   through [`blobs::place`] one line before the `rename` that would use it.
//!   Content is verified by `iroh-blobs` as it arrives and staged through
//!   `.wobu/tmp`, so a reader never sees half a file and an interrupted transfer
//!   leaves nothing at a real path.
//!   The shell now announces and fetches every indexed original during a round,
//!   then reconciles those arrivals into the local asset index before waking the
//!   window. Thumbnails remain local derived data and are drawn on demand from
//!   the received original. What is not yet assembled by the shell is the
//!   generations half of the list: `wobu-store` has a lister for indexed
//!   originals and no public lister for
//!   `generations/<YYYY-MM>/<ULID>.json`. [`Blobs::describe`] closes the hash
//!   half of that gap; the directory listing is still owed.
//! - **#82 `SyncManager`** — [`Sessions`] is the trait it implements, and the
//!   `Router`'s abort-on-drop is why it must exist.
//! - **#83 status and presence** — [`Session::is_relayed`] now,
//!   `Connection::path_events` when a live badge is wanted.
//! - **#90 authorisation** — done: a ticket presents its grant on a second stream
//!   and the app's [`Projects::admits`] implementation checks it without changing
//!   the opening message or the `ProjectNotHeld` refusal.

mod authorization;
pub mod blobs;
pub mod endpoint;
pub mod error;
pub mod identity;
pub mod manifest;
mod opening;
pub mod ticket;

pub use blobs::{Blobs, Fetched, Offered, Unplaceable};
pub use endpoint::{Config, Projects, Reach, Session, Sessions, SyncEndpoint};
pub use error::{Error, Result};
pub use identity::{Identity, Origin};
pub use manifest::{Blob, Counts, Exchange};
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
