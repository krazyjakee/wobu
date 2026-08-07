//! Moving file content: the conflict-free half of sync, and the only place in
//! this crate that touches a filesystem.
//!
//! [#81](https://github.com/krazyjakee/wobu/issues/81). [`manifest::exchange`]
//! ended with two lists of [`Blob`] — `(rel_path, hash)` for every file under
//! `assets/` and `generations/` each side holds. This module is what closes the
//! difference: fetch what we are missing, serve what they are missing, and never
//! merge anything, because there is nothing here that can conflict.
//!
//! That last claim is the issue's and it is worth restating as the invariant it
//! is. `assets/originals/<hh>/<hash>.<ext>` is named after the BLAKE3 of its own
//! bytes, so two people importing the same reference write **identical bytes to
//! an identical path**; `generations/<YYYY-MM>/<ULID>.json` is write-once and
//! ULID-named, so two people never write the same one at all. A file that is
//! present is therefore a file that is finished, and "do I have it" is the whole
//! of the decision. If this module ever grows a merge path, a conflict path or a
//! `.conflict-*` sibling, something upstream has broken the write-once rule and
//! *that* is the bug — not this.
//!
//! ## Why `iroh-blobs` rather than a third frame on `wobu/sync/1`
//!
//! Hand-rolling this would have been perhaps two hundred lines: a request naming
//! a hash, a stream of bytes back, a length. It is not two hundred lines,
//! because of one sentence in [`crate`]: **if a change to this crate needs a
//! cryptographic primitive, the change is wrong.**
//!
//! A receiver that took bytes off a stream and wrote them somewhere would have
//! to find out whether it had been lied to, and the only way to find that out is
//! to hash what arrived — which puts a hash function in the crate whose entire
//! design rule is that it does not have one, in the code path where a stranger
//! chooses the input. Worse, it would have to buffer the whole blob first,
//! because a hash computed after the last byte is a hash computed after the file
//! has already been written.
//!
//! `iroh-blobs` verifies the BLAKE3 *tree* as the bytes arrive: every chunk is
//! checked against the hash that was asked for, before it is stored, and a peer
//! that serves different content under a hash fails the transfer part-way rather
//! than completing it. So the verification is real and continuous, this crate
//! computes nothing, compares nothing and stores no digest, and the rule holds
//! unbent. That is the whole argument for the dependency; the resumability, the
//! ranges and the multi-provider downloader are things we get anyway and are not
//! why it is here.
//!
//! Consequently: **nothing in this module may start hashing.** The one place it
//! looks tempting — "check the file we just placed really is what was asked
//! for" — is the place where it is most redundant, because the bytes could not
//! have been written unless they already verified.
//!
//! ## The store is a cache, it lives outside the project folder, and that is not
//! a detail
//!
//! [`Blobs::open`] takes two paths and they are not interchangeable.
//!
//! - `root` is the project folder. It is the thing that may also be a Dropbox
//!   folder, an SMB share or a Syncthing directory — `docs/07-file-shares.md` —
//!   and it is where files land.
//! - `cache` is where `iroh-blobs` keeps its redb database and its outboards,
//!   and it **must not be inside `root`**. A redb file inside a folder two
//!   machines have mounted is two processes writing one database over SMB, which
//!   is the exact failure `wobu_store::paths::index_path` already exists to
//!   avoid: the SQLite index is keyed by project ULID and kept in local app
//!   data for this reason and no other. The blob store is the same kind of thing
//!   — derived, rebuildable, deletable — and belongs in the same kind of place.
//!
//! It is a parameter rather than something computed here because `app_data_dir`
//! lives in `wobu-store`, and this crate does not depend on `wobu-store` (see
//! [`manifest`] for the argument, which is about the diff and not about paths,
//! but which this would be the first exception to). [`Blobs::open`] refuses a
//! `cache` under `root` rather than trusting the caller to have read this
//! paragraph.
//!
//! The store is not a second copy of the project. Files are imported with
//! `ImportMode::TryReference`, so anything above `iroh-blobs`' inline threshold
//! (16 KiB) stays exactly where it is and the store holds a pointer and an
//! outboard — about 1/1000th of the file's size. Referencing is normally the
//! unsafe mode, because a referenced file may be edited afterwards and the store
//! would then be serving a hash it no longer has; here it is the *safe* mode,
//! because every path this module will import is content-addressed or write-once
//! and a file that changed under us was never legitimately ours to serve. And if
//! one does change, the receiver catches it: the bytes stop matching the
//! outboard mid-transfer and the fetch fails, which is
//! `a_referenced_file_that_changed_under_the_provider_cannot_be_passed_off` in
//! `tests/blobs.rs`.
//!
//! ## A second ALPN, on the same router, on a second connection
//!
//! `iroh-blobs` brings its own ALPN (`/iroh-bytes/4`) and its own protocol
//! handler. [`Config::blobs`] hands it to [`SyncEndpoint::bind`], which registers
//! it on the same `Router` as `wobu/sync/1` — one endpoint, one key, one socket,
//! two protocols.
//!
//! It is deliberately *not* run over the [`Session`]'s connection. TLS negotiates
//! one ALPN per connection, so blobs travelling on a `wobu/sync/1` connection
//! would mean re-implementing the request half of `iroh-blobs` against streams we
//! opened ourselves — which is the hand-rolled protocol above, wearing a
//! dependency as a hat. [`Blobs::fetch`] opens its own connection to the same
//! peer, and [`Session::addr`] exists to say where that peer currently is.
//!
//! ## Every path is validated here, at the join, and not because it arrived
//! unvalidated
//!
//! [`manifest::is_syncable_rel_path`] already ran on everything in an
//! [`Exchange`]. This module runs it again, and then four more checks on top, and
//! the reason is written in `manifest.rs`: a caller relying on a check performed
//! in another module is a check that stops being performed the day somebody adds
//! a second caller. [`place`] is that second performance, it is immediately
//! before the only `PathBuf` in this crate that becomes a real file, and it is
//! the single most security-relevant function in the crate. See its
//! documentation for what each rule stops.
//!
//! ## Nothing is ever written where a reader could see half of it
//!
//! Content is exported to `<root>/.wobu/tmp/<ULID>.part` and `rename`d into
//! place. Three properties, all load bearing:
//!
//! - `.wobu/tmp` is on the **same filesystem** as the target, so the rename is a
//!   rename and not a copy. `wobu_store::atomic` stages there for exactly this
//!   reason and says so; using the OS temp directory would silently degrade into
//!   a cross-device copy, which is not atomic.
//! - A dropped fetch — a cancelled sync, a quit, a killed process — leaves a
//!   `.part` and never a truncated asset. `Project::sweep` already deletes
//!   `.part` files in that directory on open, so the litter has an owner and it
//!   is not this crate.
//! - The target is never opened for writing at all, so a reader that opens
//!   `assets/originals/ab/<hash>.png` sees either nothing or the whole file. A
//!   thumbnailer racing a sync is the ordinary case, not the exotic one.
//!
//! ## What is counted rather than reported
//!
//! [`Fetched`] and [`Offered`] carry numbers. The paths behind those numbers are
//! strings a peer wrote, and `Exchange::refused` gives the argument for keeping
//! them out of anything a caller might log. The one exception is
//! [`Fetched::placed`], which is a list of paths — and it is safe precisely
//! because they are the paths that *passed* [`place`], which is to say the ones
//! this build constructed rather than the ones a peer proposed. #82 needs it to
//! tell `wobu-store` what appeared.
//!
//! ## What this module does not do
//!
//! - **It does not choose what to fetch.** [`Blobs::fetch`] takes a list. #81
//!   wants thumbnails eagerly and originals on demand, and that policy belongs
//!   to whatever is drawing the library grid, not to a transport. The list is
//!   the seam; a filter here would be a policy nothing could override.
//! - **It does not delete.** A blob we hold and the peer does not is a blob the
//!   peer never had — the standing rule in [`crate`], applied to files. Nothing
//!   here removes anything from `root`, ever.
//! - **It does not push.** Both sides call [`Blobs::fetch`], each pulling what it
//!   is missing, which is symmetric and needs no agreement about who goes first.
//!   `iroh-blobs` can push; using it would mean one side deciding what the other
//!   ought to want.
//!
//! [`Blob`]: crate::manifest::Blob
//! [`Exchange`]: crate::manifest::Exchange
//! [`manifest`]: crate::manifest
//! [`manifest::exchange`]: crate::manifest::exchange
//! [`manifest::is_syncable_rel_path`]: crate::manifest::is_syncable_rel_path
//! [`Session`]: crate::endpoint::Session
//! [`Session::addr`]: crate::endpoint::Session::addr
//! [`SyncEndpoint::bind`]: crate::endpoint::SyncEndpoint::bind
//! [`Config::blobs`]: crate::endpoint::Config::blobs

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use iroh::{Endpoint, EndpointAddr};
use iroh_blobs::BlobsProtocol;
use iroh_blobs::api::blobs::{AddPathOptions, ImportMode};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};

use crate::error::{Error, Result};
use crate::manifest::{Blob, is_content_hash, is_syncable_rel_path};

/// The ALPN blobs travel on, re-exported rather than re-declared.
///
/// `iroh-blobs`' own constant, so the version of the blob protocol is the
/// version `iroh-blobs` implements and there is no second place for it to be
/// stated wrongly. It is `/iroh-bytes/4` today and this crate has no opinion
/// about that; [`crate::ALPN`] is ours and this one is not.
pub const ALPN: &[u8] = iroh_blobs::ALPN;

/// The suggested bound on a single blob's transfer.
///
/// A default, not a policy, for the same reason [`crate::manifest::IDLE_TIMEOUT`]
/// is one: #82 knows whether a user is watching. It bounds **one file**, not a
/// whole fetch, because a hundred files that each take a second is a working
/// sync and a single file that takes two minutes is a stall. The spike measured
/// ~1 MB/s over a relay, so this is roughly a hundred-megabyte asset on the
/// worst link it recorded.
pub const BLOB_TIMEOUT: Duration = Duration::from_secs(120);

/// LoRA weights commonly exceed the roughly 100 MB relay budget behind
/// [`BLOB_TIMEOUT`]. The manifest carries no byte size, so the exact validated
/// path is the only honest signal available before fetching.
pub const LORA_BLOB_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub fn timeout_for(rel_path: &str, ordinary: Duration) -> Duration {
    if lora_hash_from_path(rel_path).is_some() { ordinary.max(LORA_BLOB_TIMEOUT) } else { ordinary }
}

/// The staging directory, relative to the project root.
///
/// The same one `wobu_store::atomic` uses, and the same `.part` suffix, so that
/// `Project::sweep` cleans up after an interrupted fetch without knowing this
/// crate exists. Two staging directories would mean two sweepers.
const TMP_DIR: &str = ".wobu/tmp";

/* ── the path join, which is the part to get right ────────────────────────── */

/// Why a `rel_path` will not be turned into a file.
///
/// Split into variants rather than collapsed into one, because these are the
/// assertions of `tests/blobs.rs` and a single `Refused` would let a test pass
/// for the wrong reason — a traversal caught by the length check is not a
/// traversal check that works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unplaceable {
    /// Failed [`is_syncable_rel_path`]: not under `assets/` or `generations/`,
    /// too long, non-ASCII, a control character, a backslash, a colon, an empty
    /// segment, or a `.` or `..` segment.
    NotSyncable,
    /// A segment that the host's own path parser does not read as one ordinary
    /// name. On Windows this is where `C:`, `\\?\` and `\\server\share` are
    /// caught, on any host it is the backstop for whatever the string split did
    /// not anticipate.
    NotOneSegment,
    /// A segment ending in `.` or a space. Windows strips both when it opens a
    /// file, so `x.png.` and `x.png ` are two more spellings of `x.png` — three
    /// paths that are distinct to this code and one file to the filesystem.
    TrailingDotOrSpace,
    /// A DOS device name: `CON`, `NUL`, `COM3`, and friends, with or without an
    /// extension. On Windows opening one of these talks to a device rather than
    /// creating a file.
    ReservedDeviceName,
    /// The assembled path is not under the root. Unreachable if the rules above
    /// hold, which is exactly why it is checked.
    EscapesRoot,
    /// A directory on the way to the target already exists and is a symbolic
    /// link, or is not a directory at all.
    SymlinkedAncestor,
    /// Something is already at the target and it is not a plain file.
    TargetIsNotAFile,
}

/// Turn a peer's `rel_path` into an absolute path, lexically, or refuse it.
///
/// **This is the sharpest edge in the crate.** Everything else here fails by not
/// syncing; this fails by writing a stranger's bytes to a path a stranger chose.
/// So it is a whitelist, it is checked twice, and every rule below is a real
/// escape rather than a tidiness preference.
///
/// In order:
///
/// 1. [`is_syncable_rel_path`], **again**, having already run in
///    [`crate::manifest`]. Not redundancy: that check is one module away from
///    this join and `manifest.rs` says in as many words that it is not
///    sufficient on its own. If it is ever relaxed — to allow `nodes/`, or
///    non-ASCII, or a longer path — it is relaxed here too, in front of the join,
///    where the consequence is visible.
/// 2. Per segment, the host's own parser: `Path::new(segment).components()` must
///    yield exactly one [`Component::Normal`] equal to the segment. This is the
///    check that does not depend on this author having thought of the trick. On
///    Windows `Path::new("C:")` is a `Prefix`, `Path::new("\\\\?\\x")` is a
///    verbatim prefix, and neither is `Normal` — and neither was reachable
///    anyway, because step 1 refuses `:` and `\`. Belt, braces, and a third
///    thing.
/// 3. No segment ending in `.` or a space. Win32 strips them on open, so
///    `.../ab/x.png.` and `.../ab/x.png` are one file to the OS and two paths to
///    us — which means a peer could write over a file whose path we would have
///    refused by sending a path we accept.
/// 4. No DOS device names. `assets/originals/ab/NUL.png` is not a file on
///    Windows; it is the null device, and writing "an asset" to it silently
///    succeeds and produces nothing. No real asset path can collide: originals
///    are hex, generations are ULIDs, and `CON` is neither.
/// 5. Assemble by **pushing one segment at a time**, never `root.join(rel_path)`.
///    `Path::join` with a string starting `/` or `C:\` discards the root
///    entirely and returns the argument — the single most common way this
///    function is got wrong, and the reason the assembly is spelled out rather
///    than expressed in one line.
/// 6. `strip_prefix(root)`, and every remaining component must be `Normal`. A
///    tautology if 1–5 held, which is the point: it is the assertion that says so
///    if one of them stops holding.
///
/// What is deliberately *not* here: `canonicalize` on the target. It fails on a
/// path that does not exist yet, which is every path this function is asked
/// about, and canonicalising the parent instead resolves symlinks rather than
/// refusing them. [`place`] does the symlink work, on the directories, against
/// the filesystem.
pub fn join(root: &Path, rel_path: &str) -> std::result::Result<PathBuf, Unplaceable> {
    if !is_syncable_rel_path(rel_path) {
        return Err(Unplaceable::NotSyncable);
    }

    let mut absolute = root.to_path_buf();
    for segment in rel_path.split('/') {
        let mut parsed = Path::new(segment).components();
        match (parsed.next(), parsed.next()) {
            (Some(Component::Normal(only)), None) if only == segment => {}
            _ => return Err(Unplaceable::NotOneSegment),
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(Unplaceable::TrailingDotOrSpace);
        }
        if is_reserved_device_name(segment) {
            return Err(Unplaceable::ReservedDeviceName);
        }
        // One segment at a time. Never the whole string.
        absolute.push(segment);
    }

    let inside = absolute
        .strip_prefix(root)
        .is_ok_and(|rest| rest.components().all(|c| matches!(c, Component::Normal(_))));
    if !inside {
        return Err(Unplaceable::EscapesRoot);
    }
    Ok(absolute)
}

/// [`join`], plus the questions only the filesystem can answer.
///
/// A lexical join proves the *string* stays under the root. It proves nothing
/// about the directories, and a project folder is a shared folder: on a Dropbox
/// or SMB mount somebody else can put a symbolic link in it, and
/// `assets/originals -> /home/you/.ssh` turns a perfectly well-formed relative
/// path into a write outside the project. So every directory between the root
/// and the target that already exists must be a real directory, checked with
/// `symlink_metadata`, which does not follow links.
///
/// The target itself must be absent or a plain file. A rename over a symlink
/// replaces the link rather than following it, so this is not strictly an escape
/// — but a symbolic link where a content-addressed asset should be is somebody
/// else's arrangement and quietly replacing it is not this crate's call.
///
/// **This is a check, not a lock**, and the window between it and the `rename` is
/// real. It is not closed here because closing it properly means resolving the
/// whole path with `openat`/`O_NOFOLLOW` one component at a time, which is a
/// Unix-only construction with no Windows equivalent worth the name — and the
/// attacker it would stop is somebody who already has write access to the
/// project folder, which is somebody who can simply write the file. The peer on
/// the other end of the connection, who is the attacker this module is actually
/// defending against, cannot create a symlink here at all.
pub fn place(root: &Path, rel_path: &str) -> std::result::Result<PathBuf, Unplaceable> {
    let absolute = join(root, rel_path)?;

    // Every directory from the root down to (not including) the target.
    let mut walked = root.to_path_buf();
    let parent_segments = rel_path.split('/').count().saturating_sub(1);
    for segment in rel_path.split('/').take(parent_segments) {
        walked.push(segment);
        match fs::symlink_metadata(&walked) {
            // Not there yet; we will create it, and a directory we create is not
            // a link.
            Err(_) => break,
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(Unplaceable::SymlinkedAncestor),
        }
    }

    match fs::symlink_metadata(&absolute) {
        Err(_) => Ok(absolute),
        Ok(meta) if meta.is_file() => Ok(absolute),
        Ok(_) => Err(Unplaceable::TargetIsNotAFile),
    }
}

/// Whether a `(rel_path, hash)` pair is one the *path* agrees with.
///
/// Only `assets/originals/**` has an opinion, and it has a total one:
/// `assets/originals/<first two hex>/<hash>.<ext>` is a pure function of the
/// content except the extension, so the hash in the filename and the hash on the
/// wire are the same sixty-four characters or the entry is a lie. Everything
/// else — `assets/thumbs/**` is named after the *original's* hash rather than its
/// own, `assets/meshes/**` after a directory's, `generations/**` after a ULID —
/// has no derivable relationship and returns `true` unexamined.
///
/// **This closes a real hole rather than tidying an invariant.** BLAKE3 of the
/// empty input is a hash every store can satisfy locally, without asking
/// anybody, because a zero-length blob is complete the moment it is requested. So
/// a peer that announced
/// `("assets/originals/ab/<hash of somebody's real asset>.png", <hash of
/// nothing>)` would have this module write a zero-byte file at the path that
/// asset lives at — verified, correct, and empty. Every later sync would then
/// see a file at that path, conclude it is finished (which for a
/// content-addressed tree is ordinarily sound), and never fetch the real one.
/// One line of manifest, one permanently broken asset, no error anywhere.
///
/// It is not a hash check and does not need one: both sides are strings, the
/// comparison is `==`, and the rule in [`crate`] holds.
///
/// `generations/**` has no equivalent defence and cannot have one — a ULID says
/// nothing about content — so a peer can put whatever it likes in a generation
/// record it is entitled to send at all. That is a real limit and it is smaller
/// than it looks: those files are write-once, so a peer can only ever win the
/// race for one that does not exist here yet, and losing that race means
/// receiving somebody else's generation record rather than losing our own.
pub fn agrees(rel_path: &str, hash: &str) -> bool {
    if rel_path.starts_with("assets/loras/") {
        return lora_hash_from_path(rel_path).is_some_and(|path_hash| path_hash == hash);
    }
    let Some(rest) = rel_path.strip_prefix("assets/originals/") else { return true };
    if !is_content_hash(hash) {
        return false;
    }
    let mut parts = rest.split('/');
    let (Some(shard), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
        // Under `assets/originals/` and not shaped like an original. Nothing in
        // this workspace writes such a path, so it is refused rather than waved
        // through as "no opinion".
        return false;
    };
    let stem = file.split('.').next().unwrap_or(file);
    shard == &hash[..2] && stem == hash
}

fn lora_hash_from_path(rel_path: &str) -> Option<&str> {
    let rest = rel_path.strip_prefix("assets/loras/")?;
    let mut parts = rest.split('/');
    let (Some(shard), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let hash = file.strip_suffix(".safetensors")?;
    (is_content_hash(hash) && shard == &hash[..2]).then_some(hash)
}

/// The DOS device names, which are still special in Win32 in 2026.
///
/// Matched on the part before the first `.`, case-insensitively, because
/// `NUL.png` is the null device just as much as `NUL` is. Listed rather than
/// pattern-matched on a prefix so that `COMMENT.png` — which begins `COM` — is
/// an ordinary file.
fn is_reserved_device_name(segment: &str) -> bool {
    const RESERVED: [&str; 4] = ["con", "prn", "aux", "nul"];
    let stem = segment.split('.').next().unwrap_or(segment).to_ascii_lowercase();
    if RESERVED.contains(&stem.as_str()) {
        return true;
    }
    // COM0–COM9 and LPT0–LPT9. The superscript forms Windows also accepts are
    // not reachable: `is_syncable_rel_path` refuses non-ASCII.
    matches!(stem.as_bytes(), [b'c', b'o', b'm', d] | [b'l', b'p', b't', d] if d.is_ascii_digit())
}

/* ── what a round of blob work turned up ──────────────────────────────────── */

/// What [`Blobs::offer`] made servable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Offered {
    /// Files now in the store and servable to a peer.
    pub offered: usize,
    /// Files already imported by an earlier call, whose content has not moved.
    /// Cheap: the hash was known, so nothing was read.
    pub already: usize,
    /// Entries in the caller's own list that this build will not join onto a
    /// project root. Non-zero means the *caller* is passing paths that
    /// [`place`] refuses, which is a bug on this side rather than a peer's
    /// doing.
    pub refused: usize,
    /// Entries naming a file that is not there. An index ahead of the
    /// filesystem, which is ordinary on a share that is still catching up.
    pub missing: usize,
    /// Entries whose file hashes to something other than what the caller said.
    /// The index is stale; the file is not offered under a hash it does not
    /// have.
    pub stale: usize,
}

/// What [`Blobs::fetch`] brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// The paths that are now in the project folder that were not before, in the
    /// order they landed.
    ///
    /// Safe to hand to a caller and to log, unlike anything else derived from a
    /// peer's manifest, because every one of these came out of [`place`] — they
    /// are paths this build assembled, not paths a peer proposed.
    pub placed: Vec<String>,
    /// Already present, so nothing moved. The common case on every sync after
    /// the first, and the reason a full manifest exchange is cheap.
    pub already: usize,
    /// Refused by [`place`] before a byte was asked for. **A non-zero count here
    /// is the interesting one**: [`crate::manifest`] already dropped everything
    /// syntactically wrong, so a path that survives that and fails this is a peer
    /// running a different build — or one trying something.
    pub refused: usize,
    /// The peer could not, or would not, produce the content. Counted rather
    /// than fatal: one blob a peer no longer has must not stop the other five
    /// hundred, and a file that did not arrive is a file to try for again next
    /// time.
    pub failed: usize,
}

/* ── the store ────────────────────────────────────────────────────────────── */

/// A project's blob store, and the project root it serves and fills.
///
/// Cheap to clone — `FsStore` is a handle to an actor — and the clones share one
/// store. [`crate::Config`] takes one so that [`crate::SyncEndpoint::bind`] can
/// put the blob protocol on the same router as `wobu/sync/1`.
#[derive(Debug, Clone)]
pub struct Blobs {
    store: FsStore,
    /// Canonical, so that [`place`]'s containment check is against a path the
    /// filesystem agrees with rather than one with a `..` or a symlink in it.
    root: PathBuf,
}

impl Blobs {
    /// Open the store for one project.
    ///
    /// `root` is the project folder and must already exist — this crate does not
    /// create projects. `cache` is the store's own directory, is created if
    /// missing, and **must be outside `root`**; see the module documentation for
    /// why a redb database inside a shared folder is the same mistake
    /// `wobu_store::paths::index_path` was written to avoid. The recommended
    /// value is `<app data>/blobs/<project ULID>`, matching how the SQLite index
    /// is filed, but the decision is the caller's because `app_data_dir` is in a
    /// crate this one does not depend on.
    ///
    /// One wrinkle worth stating rather than discovering: `FsStore` builds its
    /// own multi-threaded tokio runtime for its actor. [`crate`] says no runtime
    /// is created here and that remains true of this crate's own code, but it is
    /// no longer true of the process, so [`Blobs::shutdown`] is not optional
    /// housekeeping — it is how those threads stop.
    pub async fn open(root: impl AsRef<Path>, cache: impl AsRef<Path>) -> Result<Blobs> {
        let root = root.as_ref();
        let root = root.canonicalize().map_err(|source| Error::BlobStore {
            source: Box::new(std::io::Error::new(
                source.kind(),
                "the project root is not an existing directory",
            )),
        })?;
        if !root.is_dir() {
            return Err(Error::BlobStore {
                source: Box::new(std::io::Error::other("the project root is not a directory")),
            });
        }

        let cache = cache.as_ref();
        fs::create_dir_all(cache)
            .map_err(|source| Error::BlobStore { source: Box::new(source) })?;
        // Canonicalised on both sides before comparing, or `/tmp/p/../p/blobs`
        // walks straight past the check it is meant to fail.
        let canonical_cache = cache.canonicalize().unwrap_or_else(|_| cache.to_path_buf());
        if canonical_cache.starts_with(&root) {
            return Err(Error::BlobStore {
                source: Box::new(std::io::Error::other(
                    "the blob store must not live inside the project folder",
                )),
            });
        }

        let store = FsStore::load(&canonical_cache)
            .await
            .map_err(|source| Error::BlobStore { source: Box::new(source) })?;
        Ok(Blobs { store, root })
    }

    /// The project folder every path is joined onto.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The handler [`crate::SyncEndpoint::bind`] registers on [`ALPN`].
    ///
    /// `None` for the event sender: this ALPN carries neither a project id nor a
    /// ticket grant, so it has no sound per-project admission decision to make.
    /// #90 gates the sync session before a peer learns a project's manifest;
    /// blob fetches are content-addressed by hashes obtained through that
    /// admitted exchange.
    pub(crate) fn protocol(&self) -> BlobsProtocol {
        BlobsProtocol::new(&self.store, None)
    }

    /// Make the caller's list servable.
    ///
    /// The list is the same `&[Blob]` that goes into [`crate::manifest::exchange`],
    /// and passing the same slice to both is the point: **what we announce is
    /// what we can serve.** Announcing a file we have not imported costs the peer
    /// a failed fetch, and importing a file we do not announce is work nobody
    /// asked for.
    ///
    /// Idempotent and cheap to repeat. A file whose hash the store already holds
    /// is not re-read, so calling this before every exchange costs one metadata
    /// lookup per blob rather than a rehash of the project.
    ///
    /// The paths are validated with [`place`] even though they came from the
    /// caller and not from the wire. They are on their way to becoming an
    /// `add_path` — a *read* of an arbitrary file — and the day something starts
    /// echoing a peer's list back into this one, the check that stops it should
    /// already be here rather than needing to be added in a hurry.
    pub async fn offer(&self, blobs: &[Blob]) -> Result<Offered> {
        let mut tally = Offered::default();

        for blob in blobs {
            let Some(hash) = parse_hash(&blob.hash) else {
                tally.refused += 1;
                continue;
            };
            let Ok(absolute) = place(&self.root, &blob.rel_path) else {
                tally.refused += 1;
                continue;
            };
            if !agrees(&blob.rel_path, &blob.hash) {
                tally.refused += 1;
                continue;
            }
            // By hash, not by path, because serving is by hash: a digest this
            // store already holds is one it can already answer for, whoever asked
            // and under whatever name. It is also what makes calling this before
            // every exchange cost a metadata lookup rather than a re-read of the
            // project.
            //
            // The cost is that a file imported and then deleted still counts here
            // — the store holds a reference to a path that has gone. That shows
            // up as a peer's failed fetch rather than as a wrong answer, and the
            // alternative is stat-ing every file on every exchange to catch a
            // case that only arises when somebody deletes a content-addressed
            // asset by hand.
            if self.store.blobs().has(hash).await.unwrap_or(false) {
                tally.already += 1;
                continue;
            }
            if !absolute.is_file() {
                tally.missing += 1;
                continue;
            }

            let imported = self
                .store
                .blobs()
                .add_path_with_opts(AddPathOptions {
                    path: absolute,
                    format: BlobFormat::Raw,
                    // See the module documentation: the mode that is normally
                    // unsafe is the safe one here, because everything under
                    // `assets/` is named after its own bytes and everything under
                    // `generations/` is write-once.
                    mode: ImportMode::TryReference,
                })
                .await;

            match imported {
                // The file is not what the caller's index says it is. Not an
                // error and not offered: serving it under the announced hash is
                // impossible, and serving it under its real one would be
                // answering a question nobody asked.
                Ok(tag) if tag.hash != hash => tally.stale += 1,
                Ok(_) => tally.offered += 1,
                // A file that vanished between `is_file` and the import, or one
                // this process cannot read. Both are "not offered this time".
                Err(_) => tally.missing += 1,
            }
        }

        Ok(tally)
    }

    /// Hash one file in the project folder and make it servable, returning the
    /// [`Blob`] a manifest would carry for it.
    ///
    /// [`Blobs::offer`] is for the list that already has hashes in it —
    /// `wobu_store::Index::list_assets` holds the BLAKE3 of every original,
    /// because it is what the filename is made of. This is for the files that
    /// have no such row, and today that is all of `generations/**`: a generation
    /// is `generations/<YYYY-MM>/<ULID>.json`, which is not content-addressed, is
    /// not in the index, and therefore has no hash anywhere until something reads
    /// it. Rather than make the shell hash it — and give a second crate a hash
    /// function to solve a problem this one has already solved — it can walk the
    /// directory and call this.
    ///
    /// `Ok(None)` for a path [`place`] refuses or a file that is not there.
    /// Neither is an error: a directory walk that raced a delete is ordinary, and
    /// a caller cannot do anything about either except leave it out of the
    /// manifest, which is what `None` says.
    ///
    /// The hash is `iroh-blobs`', computed while importing. This crate still does
    /// not have one.
    pub async fn describe(&self, rel_path: &str) -> Result<Option<Blob>> {
        let Ok(absolute) = place(&self.root, rel_path) else { return Ok(None) };
        if !absolute.is_file() {
            return Ok(None);
        }
        let imported = self
            .store
            .blobs()
            .add_path_with_opts(AddPathOptions {
                path: absolute,
                format: BlobFormat::Raw,
                mode: ImportMode::TryReference,
            })
            .await;
        Ok(match imported {
            Ok(tag) => Some(Blob { rel_path: rel_path.to_string(), hash: tag.hash.to_hex() }),
            Err(_) => None,
        })
    }

    /// Fetch every blob in `wanted` that is not already here, from one peer.
    ///
    /// `wanted` is a list, not a policy: #81 wants `assets/thumbs/` eagerly and
    /// originals on demand, and the caller is what knows which of those this call
    /// is. Handing the whole of a peer's [`crate::manifest::Exchange::blobs`] in
    /// is legitimate and is what a "download everything" button would do; it is
    /// just not what a first open should do to a four-gigabyte project.
    ///
    /// **Cancellable by dropping the future.** Whatever was in flight is
    /// abandoned mid-stream and leaves a `.part` in `.wobu/tmp`, never a
    /// truncated file at a real path. `blobs` that had already landed stay
    /// landed, because each one is renamed into place as it finishes rather than
    /// at the end.
    ///
    /// One connection for the whole list, opened on [`ALPN`] and closed on the
    /// way out. `peer` wants network paths in it — [`crate::Session::addr`] is
    /// where a live session's come from — because a bare [`iroh::EndpointId`]
    /// needs an address-lookup service and there is not always one.
    ///
    /// Errors only when the peer cannot be reached at all. A blob that fails
    /// individually is counted in [`Fetched::failed`] and the rest continue: one
    /// file a peer has since deleted must not cost the other five hundred, which
    /// is `wobu_store::apply::Refused`'s rule and this crate's everywhere else.
    pub async fn fetch(
        &self,
        endpoint: &Endpoint,
        peer: impl Into<EndpointAddr>,
        wanted: &[Blob],
        per_blob: Duration,
    ) -> Result<Fetched> {
        let peer = peer.into();
        let id = peer.id;
        let mut tally = Fetched::default();

        // Everything that can be refused without touching the network is refused
        // first, so a manifest full of hostile paths costs a validation pass and
        // not a dial.
        let mut targets: Vec<(Hash, String)> = Vec::new();
        for blob in wanted {
            let Some(hash) = parse_hash(&blob.hash) else {
                tally.refused += 1;
                continue;
            };
            let Ok(absolute) = place(&self.root, &blob.rel_path) else {
                tally.refused += 1;
                continue;
            };
            // The path and the hash have to be talking about the same file. See
            // [`agrees`]: without this, one line of a peer's manifest can leave a
            // permanently empty file where a real asset belongs.
            if !agrees(&blob.rel_path, &blob.hash) {
                tally.refused += 1;
                continue;
            }
            // Present is finished. `assets/` is content-addressed and
            // `generations/` is write-once, so there is no version of "present
            // but out of date" for either — see the module documentation, and if
            // that stops being true this line is the bug.
            if absolute.is_file() {
                tally.already += 1;
                continue;
            }
            targets.push((hash, blob.rel_path.clone()));
        }
        if targets.is_empty() {
            return Ok(tally);
        }

        let connection = endpoint
            .connect(peer, ALPN)
            .await
            .map_err(|source| Error::Dial { peer: id, source })?;

        for (hash, rel_path) in targets {
            match self
                .fetch_one(&connection, hash, &rel_path, timeout_for(&rel_path, per_blob))
                .await
            {
                Ok(()) => tally.placed.push(rel_path),
                Err(()) => tally.failed += 1,
            }
        }

        connection.close(iroh::endpoint::VarInt::from_u32(0), b"done");
        Ok(tally)
    }

    /// One blob: verify-as-you-go into the store, then stage and rename.
    ///
    /// The error type is `()` on purpose. Every failure in here is "this file did
    /// not arrive, try again next sync" — a peer that no longer has it, a stream
    /// that died, a disk that is full, or bytes that stopped matching the hash
    /// part-way — and a caller cannot act differently on any of them. What a
    /// caller *can* act on is the count, which is what [`Fetched`] carries. The
    /// distinctions are worth a `tracing` line and nothing more, and the strings
    /// in them are attacker-adjacent.
    async fn fetch_one(
        &self,
        connection: &iroh::endpoint::Connection,
        hash: Hash,
        rel_path: &str,
        per_blob: Duration,
    ) -> std::result::Result<(), ()> {
        let fetch = self.store.remote().fetch(connection.clone(), hash);
        // The timeout is per blob rather than per fetch, and it bounds the whole
        // of one transfer rather than silence within it. That is a weaker
        // guarantee than `manifest`'s idle deadline and it is deliberate: a
        // trickle of verified bytes from a peer on a bad link is progress, but a
        // blob is a bounded thing and a caller waiting on one wants an answer.
        match tokio::time::timeout(per_blob, fetch).await {
            Ok(Ok(_stats)) => {}
            // Includes the case this module cares most about: a provider whose
            // bytes stopped matching the hash. `iroh-blobs` fails the transfer
            // where the tree stops verifying, so nothing reaches the code below.
            Ok(Err(_)) | Err(_) => return Err(()),
        }
        self.stage_and_rename(hash, rel_path).await
    }

    /// Export into `.wobu/tmp`, then `rename` onto the target.
    ///
    /// The temp file is on the same filesystem as the target because
    /// `.wobu/tmp` is inside the project root and the target is too — that is the
    /// whole reason the staging directory is not the OS one, and
    /// `wobu_store::atomic` makes the same argument at the top of its module.
    ///
    /// `sync_all` before the rename. Without it the rename can be durable while
    /// the content is not, which on a power cut leaves a correctly named,
    /// correctly sized, zero-filled asset — a file that looks present to every
    /// later `is_file` check in this module and can therefore never be fetched
    /// again. That is a worse outcome than a missing file by some distance.
    ///
    /// [`place`] runs **again**, here, on the far side of a transfer that may
    /// have taken minutes. The first call decided whether to spend bandwidth on
    /// this entry at all; this one is the check the `rename` on the next line
    /// depends on, and the two are not the same question because the filesystem
    /// can have changed in between — on a share, by somebody else. It shrinks
    /// the window in which a symbolic link could be swapped in from "the whole
    /// download" to "between two adjacent syscalls". Closing it entirely needs
    /// `openat`/`O_NOFOLLOW` per component, which has no Windows equivalent
    /// worth the name; see [`place`] for why the line is drawn there.
    async fn stage_and_rename(&self, hash: Hash, rel_path: &str) -> std::result::Result<(), ()> {
        let tmp_dir = self.root.join(TMP_DIR);
        fs::create_dir_all(&tmp_dir).map_err(|_| ())?;
        // `<ULID>.part`, which is exactly what `wobu_store::atomic` writes and
        // what `Project::sweep` already deletes on open. A second naming
        // convention would be a second thing to clean up.
        let staged = tmp_dir.join(format!("{}.part", wobu_core::new_id()));

        let exported = self.store.blobs().export(hash, &staged).await;
        let landed = exported.map_err(|_| ()).and_then(|_size| {
            fs::File::options()
                .write(true)
                .open(&staged)
                .and_then(|file| file.sync_all())
                .map_err(|_| ())
        });
        if landed.is_err() {
            let _ = fs::remove_file(&staged);
            return Err(());
        }

        // The second validation, on the far side of the transfer, immediately in
        // front of the rename that uses it.
        let Ok(absolute) = place(&self.root, rel_path) else {
            let _ = fs::remove_file(&staged);
            return Err(());
        };

        // Created after the export rather than before it, so a transfer that
        // fails leaves no empty `assets/originals/<hh>/` behind for
        // `wobu_store::assets::scan` to walk.
        if let Some(parent) = absolute.parent()
            && fs::create_dir_all(parent).is_err()
        {
            let _ = fs::remove_file(&staged);
            return Err(());
        }
        if fs::rename(&staged, &absolute).is_err() {
            let _ = fs::remove_file(&staged);
            return Err(());
        }
        Ok(())
    }

    /// Stop the store's actor and the threads it runs on.
    ///
    /// Not optional. `FsStore` builds its own multi-threaded runtime — see
    /// [`Blobs::open`] — and dropping the last handle without this leaves it to
    /// be torn down at an unspecified moment, which for a runtime dropped from
    /// inside another runtime's task is the shape of a hang at quit. #82 owns the
    /// call, beside [`crate::SyncEndpoint::shutdown`] and after it: the router
    /// must stop accepting blob connections before the store they read from goes
    /// away.
    pub async fn shutdown(&self) -> Result<()> {
        self.store.shutdown().await.map_err(|source| Error::BlobStore { source: Box::new(source) })
    }
}

/// A manifest hash string as an `iroh-blobs` hash, or nothing.
///
/// [`is_content_hash`] first, and it is not redundant with `Hash::from_str`:
/// that accepts base32 as well as hex and upper-cases before decoding, so
/// `AF13…` and `af13…` would both parse where this workspace's manifest permits
/// only one spelling of a digest. Two spellings of one hash is
/// `is_content_hash`'s argument, and letting a second parser undo it here would
/// mean a blob fetched under a hash that the exchange one module over would have
/// dropped.
fn parse_hash(hash: &str) -> Option<Hash> {
    is_content_hash(hash).then(|| Hash::from_str(hash).ok()).flatten()
}
#[cfg(test)]
mod tests;
