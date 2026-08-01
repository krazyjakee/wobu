//! The manifest exchange: what each side holds, stated in full, before a byte of
//! content moves.
//!
//! [#79](https://github.com/krazyjakee/wobu/issues/79). Two peers that have
//! agreed which project they are about — that is what a [`Session`] is — now have
//! to work out what to do about it. The cheapest honest way is for each to say
//! what it has: one `(node id, hash)` pair per node, one `(rel_path, hash)` pair
//! per blob under `assets/` and `generations/`, paged, in both directions at
//! once. [`exchange`] is that, and it is the whole of this module.
//!
//! ## This module does not decide anything
//!
//! It moves two lists and hands them back. The comparison — which node to fetch,
//! which to push, which is a conflict, which is somebody's deletion — is
//! `wobu_store::apply::decide` and `Project::plan_against_peer`, landed by #80,
//! and **it must not be re-derived here**. There would then be two answers to the
//! most consequential question in the milestone, they would drift, and the one in
//! the transport crate would be the one with no filesystem to test against.
//!
//! That is also the whole reason this crate does not depend on `wobu-store` and
//! must not gain the dependency. The seam is a slice: `Project::manifest()`
//! returns `Vec<(Id, String)>` and `Project::plan_against_peer` takes
//! `&[(Id, String)]`, so the node half of an exchange goes in and comes out with
//! **no conversion at either end** — the two types involved are `wobu_core::Id`
//! and `String`, and both crates already have them. A `NodeEntry` struct in
//! `wobu-core` would replace one shared shape with two conversions and would tie
//! this wire format's version, which is the ALPN, to
//! [`wobu_core::SCHEMA_VERSION`], which is the on-disk format's. Those are two
//! clocks and they must not be wired together.
//!
//! ## Absence means "never had it". It does not mean "deleted".
//!
//! **This is the thing to get wrong, so it is written down twice.** A node id in
//! our manifest and not in the peer's is ambiguous: they may have deleted it, or
//! they may never have had it. M3 has no tombstones, so there is nothing on the
//! wire that can tell those apart, and this exchange therefore reads every
//! absence as the second one.
//!
//! The consequence is that **a delete does not propagate**. Somebody removes a
//! node on one machine, syncs, and it is still on the other machine; sync again
//! and it may well come back. That is a known, deliberate and documented
//! shortcoming, and the alternative is worse by a margin that is not close: a
//! delete driven by absence turns a half-copied folder, a share that failed to
//! mount, or a manifest this module truncated into a world-wide erase of somebody
//! else's writing. A delete that quietly comes back is an annoyance. A delete
//! that quietly happens is unrecoverable. `wobu_store::apply::Decision::Deleted`
//! is the same decision made on the same grounds one crate over.
//!
//! Nothing may be added here to "improve" this. A deletion needs a record that
//! says a deletion happened — a tombstone, replicated, with its own rules about
//! when it may be forgotten — and that is a schema change, not a wire change.
//!
//! ## A full swap, not set reconciliation
//!
//! Every entry, every time, in both directions. A Wobu project is a few hundred
//! nodes; at that size the entire manifest is tens of kilobytes and one round
//! trip, which is less than the *first* round trip of any range-based
//! reconciliation scheme. Building one would be paying a permanent complexity
//! cost — recursive range splitting, a hash-of-hashes to maintain, and a
//! convergence argument to get right — to save bandwidth that is not being spent.
//!
//! What makes that judgement safe to revisit is that it is invisible from
//! outside: [`exchange`] returns two lists, and something cleverer that returned
//! the same two lists would be a drop-in. So the rule is not "never optimise
//! this", it is "do not optimise this until a real project makes it hurt", and
//! `docs/09-roadmap.md` is where that would be argued rather than here.
//!
//! ## The node manifest carries no path, and the blob manifest carries nothing
//! else
//!
//! #79 asks for `node_id -> (rel_path, hash)`. The `rel_path` is not on the wire
//! for nodes, and leaving it out is a decision rather than an omission.
//!
//! `wobu_store::apply::apply` places an incoming node at the path *its own index*
//! holds for that id, and only falls back to the payload's `slug` for a node it
//! has never seen — validated with [`wobu_core::is_valid_slug`] immediately
//! before the join, which is the only thing standing between a peer and a file
//! named `../../../.ssh/authorized_keys`. Its own comment on the matter: letting
//! a peer relocate our files is a much larger permission than letting it edit
//! them. So a `rel_path` in the node manifest would be a field the receiver
//! either ignores — forty wasted bytes per node — or *uses*, at which point it is
//! a second, earlier, unvalidated route to a filesystem path, arriving in a
//! message nothing parses into a node. There is no third option, and neither of
//! those two is worth having.
//!
//! Blobs are the other way round, which is why [`Blob`] has the field. An asset
//! lives at `assets/originals/<first two hex>/<hash>.<ext>` — the path is a pure
//! function of the content *except for the extension*, which is not recoverable
//! from the bytes — and a generation lives at
//! `generations/<YYYY-MM>/<ULID>.json`, which is not content-addressed at all. So
//! for blobs the path is the identity, the hash is the "do I already have it",
//! and #81 needs both. It is checked with [`is_syncable_rel_path`] before it is
//! handed to anybody, and that check is *not* sufficient on its own: #81 still
//! has to validate on the near side of its own join, because a caller relying on
//! a check performed in a different crate is a check that stops being performed
//! the day somebody adds a second caller.
//!
//! ## Framing: lines, not lengths
//!
//! The opening exchange finished its stream and used end-of-stream as the frame,
//! because a length prefix is a number an unauthenticated peer chooses. This
//! stream carries many messages, so it needs a delimiter, and it uses the same
//! reasoning to pick one: **one JSON object per line**. `serde_json` escapes
//! every newline inside a string, so a raw `\n` cannot occur inside a page and is
//! an unambiguous terminator; the reader's bound is [`MAX_PAGE_BYTES`], which is
//! this build's number, not the sender's. Nothing anywhere reads a length a peer
//! wrote.
//!
//! ## One stream each way, opened by whoever writes it
//!
//! Each side opens a *unidirectional* stream, writes its whole manifest down it,
//! and reads the peer's off the one the peer opened. Two reasons, both load
//! bearing:
//!
//! - **It is symmetric.** A bidirectional stream has an opener and an accepter,
//!   so the caller would have to know which end of the connection it is — and
//!   `SyncEndpoint::connect` and `Sessions::opened` hand out the same
//!   [`Session`] type. Getting that backwards is a hang, not a compile error.
//! - **It cannot deadlock.** Both halves run at once under `try_join!`. A
//!   half-duplex swap — write everything, then read — deadlocks the moment both
//!   manifests exceed the peer's stream receive window, which at the cap is
//!   megabytes and therefore not hypothetical.
//!
//! There is a second, sharper ordering trap inside that, and it is worth naming
//! because it was written the wrong way round first and every test failed
//! identically: **`accept_uni` resolves when the peer's first bytes arrive, not
//! when it calls `open_uni`.** QUIC puts nothing on the wire for a stream nobody
//! has written to. So a peer that accepts before it writes is waiting for a
//! stream the other peer has also opened and also not yet written to — and since
//! both sides run the same code, that is a deadlock with *no* manifest involved
//! at all. An empty exchange fails it just as reliably as a full one, which is a
//! mercy: this is the kind of mistake that would otherwise show up only under
//! load. Hence one `open_uni` up front, and the accept inside the concurrent
//! half beside the reading it belongs to.
//!
//! The cost is a constraint on whatever comes next on this ALPN: **the first
//! unidirectional stream in each direction belongs to the manifest exchange.** A
//! later `wobu/sync/1` protocol that calls `open_uni` without agreeing who reads
//! it will be answered by this module's `accept_uni`. #81 is unaffected — blobs
//! register a second ALPN, which is a different connection.
//!
//! ## The caps, and why each number is the number
//!
//! Four bounds, and they bound different things:
//!
//! - [`PAGE_ENTRIES`] = 256 — the paging. A few hundred nodes is one or two
//!   pages; ten thousand is forty, arriving steadily, rather than one message the
//!   receiver must have whole before it can look at any of it.
//! - [`MAX_PAGE_BYTES`] = 128 KiB — the allocation. The arithmetic is pinned by
//!   `a_full_page_of_the_widest_entries_this_build_will_send_still_fits`: a full
//!   node page is ~25 KB, a full blob page with every path at
//!   [`MAX_REL_PATH`] is ~56 KB, so this is roughly double the worst thing this
//!   build can produce, and a peer that exceeds it is refused rather than
//!   accommodated.
//! - [`MAX_ENTRIES`] = 50,000 per side per section — the memory. Five times the
//!   ten-thousand-node project #79 names as the degraded case and something like
//!   a hundred times the ordinary one. But the entry count is not the number to
//!   keep in mind; the *product* is, because that is what a stranger can make
//!   this process retain. A node entry is a sixteen-byte id and a sixty-four
//!   character hash, so fifty thousand of them is about five megabytes; a blob
//!   entry is a hash and a path bounded by [`MAX_REL_PATH`], so fifty thousand is
//!   about twelve. Anything that puts a field on either of those types has
//!   multiplied it by fifty thousand, and this is the paragraph that has to be
//!   re-done when it does.
//! - `idle` — the clock, and it is a parameter because #82 owns policy. It bounds
//!   **silence, not size**: every read and every page write gets the same
//!   deadline, so a large manifest over a slow relay completes as long as it
//!   keeps arriving, and a peer that opens a stream and then says nothing is gone
//!   in seconds. A single deadline over the whole exchange would have made the
//!   cap unreachable by construction, which is a worse failure than the one it
//!   prevents. [`IDLE_TIMEOUT`] is the suggested value.
//!
//! ## Going over the cap degrades; it does not fail, and it cannot lose anything
//!
//! A manifest longer than [`MAX_ENTRIES`] is cut, not refused, and
//! [`Exchange::is_whole`] says so. Refusing would mean a project that grew past
//! the cap could never sync again — including the deletion that would bring it
//! back under.
//!
//! Cutting is safe for exactly the reason the first section is written the way it
//! is: a truncated entry arrives as an *absence*, an absence means "never had
//! it", and `decide` turns that into `SendOurs` (we offer bytes they already
//! have, and their `apply` reports `InStep`) or `Deleted` (a no-op). Neither
//! writes anything and neither removes anything. The cost of truncation is a sync
//! that does not converge on the tail; there is no version of it that costs
//! somebody's writing. Read that sentence next to the first section — they are
//! the same argument, and if either stops being true the other one has to be
//! re-examined.
//!
//! ## No cryptography, still
//!
//! Nothing here hashes anything. The hashes on the wire are strings the two index
//! files already hold — BLAKE3 hex computed by `wobu_store::atomic`, on the side
//! that owns the bytes — and this module compares them for nothing and derives
//! from them nothing. [`is_content_hash`] checks that a string is 64 lowercase
//! hex characters, which is a syntax check on a field, not a primitive. The rule
//! in [`crate`] holds: if a change here needs one, the change is wrong.

use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

use crate::endpoint::Session;
use crate::error::{Error, Result};

/* ── the numbers ──────────────────────────────────────────────────────────── */

/// Entries per page. See the module documentation for the arithmetic this sits
/// in the middle of.
///
/// Public because a test that asserts the paging boundary has to be able to name
/// it, and a test that hard-coded 256 would silently stop testing a boundary the
/// day this changed.
pub const PAGE_ENTRIES: usize = 256;

/// The ceiling on one page, and the only thing standing between a peer and an
/// allocation the peer chooses the size of.
///
/// Roughly double the widest page this build can produce. A peer that writes a
/// longer line is refused with [`Error::ManifestMalformed`] rather than
/// accommodated: growing this to fit whatever arrived would be letting the sender
/// pick the number after all.
pub const MAX_PAGE_BYTES: usize = 128 * 1024;

/// The longest `rel_path` this build will send or accept for a blob.
///
/// The real ones are shorter and not by a little: `assets/originals/ab/<64 hex
/// characters>.png` is 87 bytes and `generations/2026-07/<26 character ULID>.json`
/// is 45. This is roughly half again on top of the longest of them, which is
/// enough for a longer extension and not enough to be interesting — a bound here
/// is what keeps [`MAX_PAGE_BYTES`] a statement about entries rather than about
/// how imaginative a peer is feeling.
pub const MAX_REL_PATH: usize = 128;

/// The most entries this build will send, or retain from a peer, in one section
/// of one exchange.
///
/// Not a statement about how large a Wobu project may be — it is a bound on what
/// a stranger can make this process hold. Going over it degrades rather than
/// fails; see the module documentation for why that cannot lose anything.
pub const MAX_ENTRIES: usize = 50_000;

/// The suggested bound on **silence** during an exchange.
///
/// A default, not a policy: [`exchange`] takes the duration, because #82's
/// `SyncManager` is what knows whether this connection is a background poll or a
/// user standing in front of a progress bar. Fifteen seconds is several relayed
/// round trips on a bad link and short enough that a peer which opens a stream
/// and stops is not holding a task until the process ends.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// The number of characters in the BLAKE3 hex `wobu_store::atomic::Stamp` and
/// `wobu_core::asset` both produce.
///
/// Declared here rather than imported, because importing it would mean depending
/// on the crate whose absence is this module's architectural point. It is checked
/// against a real digest in `a_hash_is_sixty_four_lowercase_hex_characters`, so
/// the duplication cannot drift in silence.
const HASH_HEX_LEN: usize = 64;

/// How much of a peer's writing is read into the line buffer at a time. Not a
/// protocol constant; nothing on the wire depends on it.
const READ_CHUNK: usize = 16 * 1024;

/* ── what an exchange is about ────────────────────────────────────────────── */

/// One file under `assets/` or `generations/`, as a peer announces it.
///
/// The path and the hash, and nothing else, because those are the two questions
/// #81 has to answer: where does this go, and do I already have it. There is no
/// size, no mime type and no created-at — every one of those is either derivable
/// from the bytes once they arrive or is already in the node file that references
/// the asset, and a field on the wire that duplicates one somewhere else is a
/// field that can disagree with it.
///
/// A `Blob` that arrived from a peer has been through [`is_syncable_rel_path`]
/// and [`is_content_hash`]. That makes it *syntactically* placeable and not
/// *trustworthy*: see the module documentation on why #81 must still check
/// before it joins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blob {
    /// Project-relative, forward slashes, under `assets/` or `generations/`.
    pub rel_path: String,
    /// Full BLAKE3 hex of the file's bytes.
    pub hash: String,
}

/// How many of each thing, in one direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub nodes: usize,
    pub blobs: usize,
}

/// What one exchange turned up.
///
/// [`Exchange::nodes`] is deliberately the exact type
/// `wobu_store::Project::plan_against_peer` takes, so the call site is one line
/// with no adaptation in it. Everything else here is about whether that line is
/// looking at the whole picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    /// The peer's nodes, in the order it wrote them. Not sorted, and not
    /// deduplicated: the comparison sorts, and a peer sending the same id twice
    /// is a peer that wasted some bytes rather than one that broke anything.
    pub nodes: Vec<(Id, String)>,
    /// The peer's blobs. These resolve to plain fetches — a hash we do not have
    /// is a file we do not have — which is #81's work, not this module's.
    pub blobs: Vec<Blob>,
    /// What the peer *said* it holds, which is what it counted before its own cap
    /// bit. Equal to the lengths above in every ordinary exchange.
    pub held: Counts,
    /// How much of what the caller handed in actually went out. Less than it
    /// handed in exactly when this side's cap bit; the caller has the original
    /// slices and can compare.
    pub sent: Counts,
    /// Entries dropped on arrival because the hash was not a hash or the path was
    /// not one this crate will pass on.
    ///
    /// Counted rather than reported in detail, and dropped rather than fatal, for
    /// the reason `wobu_store::apply::Refused` gives: one malformed entry must
    /// not be able to stop the other two hundred good ones. A non-zero count here
    /// means a peer running a different build or a corrupt index, and it is worth
    /// a log line and nothing more — the strings themselves are attacker-chosen
    /// and do not belong in one.
    pub refused: usize,
}

impl Exchange {
    /// Whether the peer's side of this crossed entire.
    ///
    /// False when a cap bit — this build's on the way in, or the peer's on the
    /// way out — and also when a peer's counts simply do not match what it sent.
    /// One question rather than two, because the caller's response is the same in
    /// both cases and is the same as its response to any other absence: **treat
    /// what is missing as never having existed, never as deleted.** A `false`
    /// here is worth surfacing, because a sync that cannot converge should not
    /// keep quietly reporting that it did.
    pub fn is_whole(&self) -> bool {
        self.nodes.len() == self.held.nodes && self.blobs.len() == self.held.blobs
    }

    /// How much of the peer's manifest never arrived. Zero in an ordinary
    /// exchange.
    ///
    /// Saturating rather than wrapping: a peer whose counts are *lower* than what
    /// it sent is confused, not negative, and this is a diagnostic rather than an
    /// accusation.
    pub fn elided(&self) -> Counts {
        Counts {
            nodes: self.held.nodes.saturating_sub(self.nodes.len()),
            blobs: self.held.blobs.saturating_sub(self.blobs.len()),
        }
    }
}

/* ── the wire ─────────────────────────────────────────────────────────────── */

/// One line on the wire.
///
/// Internally tagged, so a page names itself and a reader written against a later
/// build can tell a section it does not understand from a corrupt one. The tag is
/// *not* a version: the ALPN is the version, exactly as it is for the opening
/// message, and a second version number in the payload would be one that can
/// disagree with the one TLS already negotiated.
///
/// [`Page::End`] is last and nothing follows it. That is what makes an exchange
/// self-delimiting without either side closing the connection, which matters
/// because the connection has #81 still to do on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "page", rename_all = "snake_case")]
enum Page {
    /// Node ids and their full BLAKE3 hex, as tuples rather than as a struct —
    /// two-element arrays on the wire, which is about a third of the bytes of
    /// `{"nodeId":…,"hash":…}` repeated fifty thousand times, and the exact shape
    /// `Project::manifest` already hands over.
    Nodes { entries: Vec<(Id, String)> },
    Blobs { entries: Vec<Blob> },
    /// The last page, carrying what this side actually holds — which is not what
    /// it sent, if its own cap bit. That difference is the only way the receiver
    /// can know it is looking at a partial picture, so it travels even when it is
    /// zero.
    End { held: Counts },
}

/* ── validation ───────────────────────────────────────────────────────────── */

/// Whether a string is a full BLAKE3 digest in the form this workspace writes.
///
/// Sixty-four lowercase hex characters. Uppercase is refused rather than folded,
/// because a hash is compared with `==` at the far end and two spellings of one
/// digest would read as two different files — better to drop the entry, which
/// costs a sync that does not converge, than to accept a hash that will never
/// match anything and be treated as a genuine difference forever.
///
/// The point of checking at all is not correctness — a wrong hash costs a wasted
/// fetch and nothing worse, because `apply` recomputes from the bytes it actually
/// receives — it is that this is where a peer's strings become strings this
/// process keeps fifty thousand of.
pub fn is_content_hash(hash: &str) -> bool {
    hash.len() == HASH_HEX_LEN
        && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether a blob path is one this crate will pass on to be joined onto a project
/// root.
///
/// Syntactic and conservative, and deliberately stated as a whitelist. `assets/`
/// and `generations/` are the two trees #79 names; `nodes/` is *not* here,
/// because a node's bytes travel as `wobu_store::apply::Incoming` with a
/// separately validated slug and a blob path reaching into `nodes/` would be a
/// second way to write a node file with none of that on it.
///
/// The rules after the prefix are the ordinary ones and each of them is a real
/// escape rather than a tidiness rule: no empty segment (`assets//../x`
/// normalises differently on different platforms), no `.` or `..` segment
/// (traversal), no backslash (a path separator on Windows and an ordinary
/// character in a POSIX filename, so a string that is one path here is two paths
/// there), no colon (drive letters and NTFS alternate data streams), no control
/// characters, and nothing non-ASCII — asset paths are hex and an extension and
/// generation paths are a date and a ULID, so there is no legitimate entry this
/// excludes.
///
/// **Not sufficient.** It runs on strings a stranger wrote, one crate away from
/// whatever eventually joins them onto a real directory, and the join is where
/// the check has to be. See `wobu_store::apply::Refused::UnusableSlug` for the
/// same argument about node slugs, made at the join.
pub fn is_syncable_rel_path(rel_path: &str) -> bool {
    if rel_path.is_empty() || rel_path.len() > MAX_REL_PATH || !rel_path.is_ascii() {
        return false;
    }
    if !(rel_path.starts_with("assets/") || rel_path.starts_with("generations/")) {
        return false;
    }
    if rel_path.bytes().any(|b| b.is_ascii_control() || b == b'\\' || b == b':') {
        return false;
    }
    rel_path.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/* ── the exchange ─────────────────────────────────────────────────────────── */

/// Swap manifests with the peer on the other end of a session.
///
/// Both sides must call this, at roughly the same time: each opens the stream it
/// writes and waits for the one it reads, so a peer that has not entered the
/// exchange is indistinguishable from a peer that has gone quiet, and `idle` is
/// what ends the wait. The natural place to call it is as soon as a [`Session`]
/// exists on either side, which is what makes "roughly the same time" true
/// without any sequencing.
///
/// `nodes` is `Project::manifest()` and goes back into
/// `Project::plan_against_peer` unchanged. `blobs` is the `assets/` and
/// `generations/` half, which the shell assembles — this crate has no filesystem
/// and must not gain one; `identity.rs` makes that argument at length and it does
/// not get weaker for being about content instead of keys.
///
/// Leaves the connection open and usable. Nothing here closes, resets or drains
/// anything belonging to anybody else, because #81 has work to do on the same
/// connection afterwards.
pub async fn exchange(
    session: &Session,
    nodes: &[(Id, String)],
    blobs: &[Blob],
    idle: Duration,
) -> Result<Exchange> {
    let connection = session.connection();

    // Opening a unidirectional stream is local — QUIC puts nothing on the wire
    // until something is written to it — so this resolves immediately and cannot
    // be what a peer is waiting on.
    let send = connection.open_uni().await.map_err(Error::interrupted)?;

    // Everything else runs at once, and the shape is load bearing in two
    // directions:
    //
    // - **Accepting must not come before writing.** `accept_uni` resolves when
    //   the peer's *first bytes* arrive, not when it calls `open_uni`. Waiting
    //   for it first would mean both peers waiting for a stream neither has
    //   written to yet: a symmetric deadlock that a test with an empty manifest
    //   fails just as reliably as one with fifty thousand entries.
    // - **Reading must not come after writing.** Writing everything before
    //   reading anything deadlocks as soon as either manifest is larger than the
    //   peer's stream receive window, which at the cap is megabytes.
    let (sent, received) = tokio::try_join!(write_manifest(send, nodes, blobs, idle), async {
        let mut recv =
            within(idle, async { connection.accept_uni().await.map_err(Error::interrupted) })
                .await?;
        read_manifest(&mut recv, idle).await
    })?;

    Ok(Exchange {
        nodes: received.nodes,
        blobs: received.blobs,
        held: received.held,
        sent,
        refused: received.refused,
    })
}

/// Our half: pages, then [`Page::End`], then wait for the peer to have it.
///
/// The wait is the same one `opening::answer` makes and for the same reason:
/// `Connection::close` may discard stream data the remote QUIC stack has received
/// but not yet delivered, so a send that returned as soon as the bytes were
/// queued would let a caller which closes promptly turn a complete manifest into
/// a truncated one. Here that would read to the peer as a project that has lost
/// half its nodes, which is precisely the absence this module refuses to treat as
/// a deletion — but it is still a sync that does nothing, repeatedly.
async fn write_manifest(
    mut send: SendStream,
    nodes: &[(Id, String)],
    blobs: &[Blob],
    idle: Duration,
) -> Result<Counts> {
    let mut sent = Counts::default();

    for chunk in nodes[..nodes.len().min(MAX_ENTRIES)].chunks(PAGE_ENTRIES) {
        write_page(&mut send, &Page::Nodes { entries: chunk.to_vec() }, idle).await?;
        sent.nodes += chunk.len();
    }
    for chunk in blobs[..blobs.len().min(MAX_ENTRIES)].chunks(PAGE_ENTRIES) {
        write_page(&mut send, &Page::Blobs { entries: chunk.to_vec() }, idle).await?;
        sent.blobs += chunk.len();
    }

    // The true counts, not `sent`. A receiver's only way to know it is looking at
    // a partial picture is that these two numbers disagree with what arrived.
    let held = Counts { nodes: nodes.len(), blobs: blobs.len() };
    write_page(&mut send, &Page::End { held }, idle).await?;

    send.finish().map_err(Error::interrupted)?;
    within(idle, async { send.stopped().await.map(|_| ()).map_err(Error::interrupted) }).await?;
    Ok(sent)
}

/// One page, one line.
///
/// The `expect` is not optimism. A [`Page`] is ids, hex strings and paths this
/// build assembled, and `serde_json` fails on exactly two things — a map with
/// non-string keys and a float that is not a number — neither of which can occur
/// in this type. Returning an error here would be a branch no test could ever
/// reach and no caller could ever act on.
async fn write_page(send: &mut SendStream, page: &Page, idle: Duration) -> Result<()> {
    let mut line = serde_json::to_vec(page).expect("a Page is ids, hex and paths");
    debug_assert!(
        line.len() < MAX_PAGE_BYTES,
        "this build produced a {} byte page, which its own peers will refuse",
        line.len()
    );
    line.push(b'\n');
    within(idle, async { send.write_all(&line).await.map_err(Error::interrupted) }).await
}

/// The peer's half, as far as [`Page::End`].
struct Received {
    nodes: Vec<(Id, String)>,
    blobs: Vec<Blob>,
    held: Counts,
    refused: usize,
}

/// Read until [`Page::End`], keeping at most [`MAX_ENTRIES`] of each section.
///
/// Entries past the cap are *read and dropped* rather than left unread. Leaving
/// them would stall the peer's write behind a receive window that never opens,
/// and it would leave this side unable to reach `End` — which is the only page
/// carrying the counts that make the truncation visible. Reading past the cap
/// costs bytes through a socket and no memory, and `idle` is what bounds a peer
/// that intends to write forever.
async fn read_manifest(recv: &mut RecvStream, idle: Duration) -> Result<Received> {
    let mut pages = Lines { recv, buf: Vec::new(), idle };
    let mut nodes: Vec<(Id, String)> = Vec::new();
    let mut blobs: Vec<Blob> = Vec::new();
    let mut refused = 0usize;

    loop {
        match pages.next_page().await? {
            Some(Page::Nodes { entries }) => {
                for (id, hash) in entries {
                    if !is_content_hash(&hash) {
                        refused += 1;
                    } else if nodes.len() < MAX_ENTRIES {
                        nodes.push((id, hash));
                    }
                }
            }
            Some(Page::Blobs { entries }) => {
                for blob in entries {
                    if !is_content_hash(&blob.hash) || !is_syncable_rel_path(&blob.rel_path) {
                        refused += 1;
                    } else if blobs.len() < MAX_ENTRIES {
                        blobs.push(blob);
                    }
                }
            }
            Some(Page::End { held }) => return Ok(Received { nodes, blobs, held, refused }),
            // The stream finished without saying it was finished. Not treated as
            // a short manifest, because a short manifest and a cut connection
            // would then be the same event — and one of them is a peer that holds
            // nothing, which is a sentence this crate is not willing to infer
            // from a dropped TCP-shaped thing.
            None => return Err(Error::ManifestMalformed),
        }
    }
}

/// Newline-delimited pages off a stream a stranger is writing.
///
/// Hand-rolled over `RecvStream::read` rather than reached for through
/// `tokio::io::AsyncBufReadExt`, because the interesting part is the bound and
/// `read_until` does not have one. The whole point is that the ceiling on a line
/// is [`MAX_PAGE_BYTES`] — this build's constant — and never a length the sender
/// wrote.
struct Lines<'a> {
    recv: &'a mut RecvStream,
    /// Bytes read and not yet consumed. Holds at most one page plus whatever of
    /// the next one arrived in the same chunk.
    buf: Vec<u8>,
    idle: Duration,
}

impl Lines<'_> {
    /// The next page, or `None` at a clean end of stream.
    async fn next_page(&mut self) -> Result<Option<Page>> {
        loop {
            if let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=end).collect();
                line.pop();
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(|_| Error::ManifestMalformed);
            }
            // Checked before reading more, so the buffer never grows past one
            // chunk beyond the cap however much the peer writes in one go.
            if self.buf.len() > MAX_PAGE_BYTES {
                return Err(Error::ManifestMalformed);
            }

            let mut chunk = [0u8; READ_CHUNK];
            let read = within(self.idle, async {
                self.recv.read(&mut chunk).await.map_err(Error::interrupted)
            })
            .await?;
            match read {
                Some(n) => self.buf.extend_from_slice(&chunk[..n]),
                // End of stream. Trailing bytes with no newline are half a page,
                // which is a peer that stopped mid-write and not an empty
                // manifest.
                None if self.buf.is_empty() => return Ok(None),
                None => return Err(Error::ManifestMalformed),
            }
        }
    }
}

/// Put the silence deadline on one step of the exchange.
///
/// Every read and every page write, rather than once around the whole thing. The
/// difference is the whole contract: this bounds how long a peer may say nothing,
/// so a fifty-thousand-node manifest over a relay finishes as long as it keeps
/// arriving. A single deadline over the exchange would have made [`MAX_ENTRIES`]
/// unreachable on any link slow enough to need a cap in the first place.
async fn within<T>(idle: Duration, step: impl Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout(idle, step).await {
        Ok(result) => result,
        Err(_elapsed) => Err(Error::ManifestTimedOut { idle }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(seed: u128) -> String {
        format!("{seed:032x}{seed:032x}")
    }

    /* ── the numbers hold together ────────────────────────────────────── */

    #[test]
    fn a_full_page_of_the_widest_entries_this_build_will_send_still_fits() {
        // The arithmetic behind `MAX_PAGE_BYTES`, asserted rather than asserted
        // in a comment. If `PAGE_ENTRIES` or `MAX_REL_PATH` is ever raised, this
        // is what says so — and it fails on the *sending* build, which is the one
        // that can be fixed, rather than on whichever peer happened to receive
        // the oversized page.
        let nodes = Page::Nodes {
            entries: (0..PAGE_ENTRIES as u128).map(|n| (Id::from(n), hex(n))).collect(),
        };
        let widest = "assets/".to_string() + &"w".repeat(MAX_REL_PATH - "assets/".len());
        assert_eq!(widest.len(), MAX_REL_PATH);
        assert!(is_syncable_rel_path(&widest));
        let blobs = Page::Blobs {
            entries: (0..PAGE_ENTRIES as u128)
                .map(|n| Blob { rel_path: widest.clone(), hash: hex(n) })
                .collect(),
        };

        for page in [nodes, blobs] {
            let line = serde_json::to_vec(&page).unwrap();
            assert!(line.len() < MAX_PAGE_BYTES, "a full page is {} bytes", line.len());
        }
    }

    #[test]
    fn a_page_is_exactly_one_line() {
        // The framing, and the reason a length prefix is not needed. `serde_json`
        // escapes newlines inside strings, so the only `\n` in a page's encoding
        // is the one `write_page` appends — which is what makes end-of-line an
        // unambiguous frame even when a peer puts a newline in a path.
        let page = Page::Blobs {
            entries: vec![Blob {
                rel_path: "assets/originals/ab/\n\r\u{2028}oops.png".into(),
                hash: hex(1),
            }],
        };

        let line = serde_json::to_vec(&page).unwrap();

        assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 0);
    }

    #[test]
    fn a_hash_is_sixty_four_lowercase_hex_characters() {
        // Pins `HASH_HEX_LEN` against a digest `wobu-store` actually produced,
        // since the constant is declared here rather than imported from the crate
        // that produces one — see `HASH_HEX_LEN` for why importing it is not an
        // option.
        let real = BLAKE3_OF_NOTHING;
        assert_eq!(real.len(), HASH_HEX_LEN);
        assert!(is_content_hash(real));

        assert!(!is_content_hash(""));
        assert!(!is_content_hash(&real[..63]));
        assert!(!is_content_hash(&format!("{real}0")));
        assert!(!is_content_hash(&real.to_uppercase()), "two spellings of one digest");
        assert!(!is_content_hash(&"g".repeat(64)));
        // 64 bytes but 63 characters: indexing bytes would split the first one in
        // half, which is the same trap `apply::endpoint_bytes` guards.
        assert!(!is_content_hash(&format!("é{}", "0".repeat(62))));
    }

    /// BLAKE3 of the empty input — the one published test vector — written out
    /// rather than computed, because computing it would mean this crate taking a
    /// dependency on a hash function, and the rule in `lib.rs` does not have a
    /// "but only in tests" clause. It is here to pin the *shape* `wobu-store`'s
    /// `atomic::hash_bytes` produces: sixty-four lowercase hex characters.
    const BLAKE3_OF_NOTHING: &str =
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    /* ── paths a stranger wrote ───────────────────────────────────────── */

    #[test]
    fn a_blob_path_has_to_be_under_assets_or_generations() {
        for good in [
            "assets/originals/ab/af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262.png",
            "assets/thumbs/ab/af1349b9.webp",
            "assets/meshes/ab/model.glb",
            "generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json",
        ] {
            assert!(is_syncable_rel_path(good), "{good} was refused");
        }

        for bad in [
            "",
            "assets",
            "assets/",
            // `nodes/` has its own path, with its own validation, on the payload
            // rather than on the manifest.
            "nodes/character/kael-vantris.md",
            "project.json",
            ".wobu/index.sqlite",
            "/etc/passwd",
            "/assets/x.png",
            "../assets/x.png",
            "assets/../../.ssh/authorized_keys",
            "assets/./x.png",
            "assets//x.png",
            // Windows: a separator here, an ordinary filename character there.
            "assets\\..\\..\\x.png",
            "assets/c:/x.png",
            "assets/x.png:stream",
            "assets/x\0.png",
            "assets/x\n.png",
            // Non-ASCII, which no real asset or generation path contains.
            "assets/café.png",
        ] {
            assert!(!is_syncable_rel_path(bad), "{bad:?} was accepted");
        }

        let too_long = format!("assets/{}", "a".repeat(MAX_REL_PATH));
        assert!(!is_syncable_rel_path(&too_long));
    }

    /* ── the shape the caller reads ───────────────────────────────────── */

    #[test]
    fn an_exchange_that_lost_nothing_says_it_lost_nothing() {
        let whole = Exchange {
            nodes: vec![(Id::from(1u128), hex(1))],
            blobs: vec![],
            held: Counts { nodes: 1, blobs: 0 },
            sent: Counts { nodes: 1, blobs: 0 },
            refused: 0,
        };

        assert!(whole.is_whole());
        assert_eq!(whole.elided(), Counts::default());
    }

    #[test]
    fn an_exchange_that_was_cut_reports_how_much_by() {
        // The degraded case, and the one thing a caller must not be able to miss:
        // a partial manifest is still a manifest full of absences, and an absence
        // is never a deletion. `is_whole` is how a caller knows not to believe it
        // has converged.
        let cut = Exchange {
            nodes: vec![(Id::from(1u128), hex(1))],
            blobs: vec![],
            held: Counts { nodes: 12_000, blobs: 3 },
            sent: Counts::default(),
            refused: 0,
        };

        assert!(!cut.is_whole());
        assert_eq!(cut.elided(), Counts { nodes: 11_999, blobs: 3 });
    }

    #[test]
    fn a_peer_that_undercounts_itself_is_a_diagnostic_and_not_an_overflow() {
        let odd = Exchange {
            nodes: vec![(Id::from(1u128), hex(1)), (Id::from(2u128), hex(2))],
            blobs: vec![],
            held: Counts { nodes: 1, blobs: 0 },
            sent: Counts::default(),
            refused: 0,
        };

        assert!(!odd.is_whole());
        assert_eq!(odd.elided(), Counts::default());
    }
}
