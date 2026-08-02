//! Guarded atomic writes.
//!
//! Node Markdown is the only file in a project that two people can edit at
//! once, so it is the only mutable thing this module protects. Every write stages into
//! `.wobu/tmp` on the **same filesystem** — using the OS temp dir would
//! silently degrade the rename into a cross-device copy, which is not atomic —
//! checks that the target has not moved under us, and then renames into place.
//! Write-once records use the same staging area, then publish with a hard link:
//! unlike `rename`, that operation can never replace a name another writer has
//! already claimed.
//!
//! We never merge. The loser's version lands beside the winner's as a
//! `.conflict-*.md` sibling and the UI raises a diff. See
//! `docs/07-file-shares.md`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A portable, project-relative path that has passed the storage boundary.
///
/// Content publishers accept this type rather than `Path`: an absolute path,
/// `..`, or a platform-specific separator must not be able to redirect an
/// immutable write outside the selected project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectRelativePath(PathBuf);

impl ProjectRelativePath {
    pub(crate) fn new(path: &str) -> Result<Self> {
        let parsed = Path::new(path);
        let valid = !path.is_empty()
            && !path.contains('\\')
            && !path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
            && !parsed.is_absolute()
            && path
                .split('/')
                .all(|component| !component.is_empty() && component != "." && component != "..")
            && parsed
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)));
        if !valid {
            return Err(Error::io(
                parsed,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "project path must be normalized and relative",
                ),
            ));
        }
        Ok(Self(parsed.to_path_buf()))
    }

    pub(crate) fn resolve(&self, project_root: &Path) -> PathBuf {
        project_root.join(&self.0)
    }
}

/// Result of publishing immutable, content-addressed bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentPublish {
    Published,
    Existing,
}

/// What validation established about a name already present at a content
/// address. Only a regular, visibly incomplete file may be removed; a valid
/// winner is immutable and suspicious same-size content is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingContent {
    Valid,
    ReplaceablePartial,
}

struct StagedFile {
    path: PathBuf,
}

impl StagedFile {
    fn new(project_root: &Path, bytes: &[u8]) -> Result<Self> {
        let tmp_dir = project_root.join(".wobu").join("tmp");
        crate::paths::ensure_dir(&tmp_dir)?;
        let path = tmp_dir.join(format!("{}.part", wobu_core::new_id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| Error::io(&path, error))?;
        let staged = file
            .write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| Error::io(&path, error));
        drop(file);
        if let Err(error) = staged {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self { path })
    }

    fn rename(mut self, target: &Path) -> Result<()> {
        fs::rename(&self.path, target).map_err(|error| Error::io(target, error))?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Validate and publish immutable bytes without ever replacing a canonical
/// file.
///
/// `validate_bytes` establishes the content/path relationship before staging;
/// `validate_existing` proves an already-present winner is a real dedupe, or
/// identifies a visibly incomplete regular file left by interrupted sync. The
/// latter may be removed and retried; valid or suspicious complete content is
/// never replaced. The
/// hard link is the key operation: on Windows it closes the rename
/// check-then-create race, and on Unix it avoids rename's overwrite semantics.
/// On SMB/NFS it is attempted on the project filesystem itself; if that share
/// cannot provide atomic hard links the operation fails and the synced staging
/// file is removed, while any canonical winner remains untouched.
pub(crate) fn publish_content_addressed(
    project_root: &Path,
    relative: &ProjectRelativePath,
    bytes: &[u8],
    validate_bytes: impl FnOnce(&[u8]) -> Result<()>,
    validate_existing: impl Fn(&Path) -> Result<ExistingContent>,
) -> Result<ContentPublish> {
    publish_content_addressed_with_link(
        project_root,
        relative,
        bytes,
        validate_bytes,
        validate_existing,
        |source, target| fs::hard_link(source, target),
    )
}

fn publish_content_addressed_with_link(
    project_root: &Path,
    relative: &ProjectRelativePath,
    bytes: &[u8],
    validate_bytes: impl FnOnce(&[u8]) -> Result<()>,
    validate_existing: impl Fn(&Path) -> Result<ExistingContent>,
    link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<ContentPublish> {
    validate_bytes(bytes)?;
    let target = relative.resolve(project_root);
    match fs::symlink_metadata(&target) {
        Ok(_) => match validate_existing(&target)? {
            ExistingContent::Valid => return Ok(ContentPublish::Existing),
            ExistingContent::ReplaceablePartial => {
                fs::remove_file(&target).map_err(|error| Error::io(&target, error))?;
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io(&target, error)),
    }
    if let Some(parent) = target.parent() {
        crate::paths::ensure_dir(parent)?;
    }
    let staged = StagedFile::new(project_root, bytes)?;
    match link(&staged.path, &target) {
        Ok(()) => Ok(ContentPublish::Published),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match validate_existing(&target)? {
                ExistingContent::Valid => Ok(ContentPublish::Existing),
                ExistingContent::ReplaceablePartial => Err(Error::io(
                    &target,
                    std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "a partial destination appeared during atomic publication",
                    ),
                )),
            }
        }
        Err(error) => Err(Error::io(&target, error)),
    }
}

/// What we believe about a file on disk. `hash` is authoritative; `mtime` and
/// `size` are the cheap pre-filter that lets the watcher skip re-reading
/// hundreds of unchanged files over SMB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub mtime_ms: i64,
    pub size: u64,
    pub hash: String,
}

impl Stamp {
    pub fn of_bytes(bytes: &[u8], mtime_ms: i64) -> Stamp {
        Stamp { mtime_ms, size: bytes.len() as u64, hash: blake3::hash(bytes).to_hex().to_string() }
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Publish complete bytes at a name that may be claimed exactly once.
///
/// Generation JSON is append-only, so an existing target is never compared,
/// updated or parked as a conflict: reusing its ULID is an error even when the
/// bytes happen to be identical. The hard link is the no-clobber equivalent of
/// the rename used by [`guarded_write`]. It exposes the fully-written, synced
/// staging inode in one filesystem operation and fails atomically when another
/// process already owns `target`.
pub fn write_once(project_root: &Path, target: &Path, bytes: &[u8]) -> Result<Stamp> {
    if let Some(parent) = target.parent() {
        crate::paths::ensure_dir(parent)?;
    }
    let staged = StagedFile::new(project_root, bytes)?;
    let published = fs::hard_link(&staged.path, target);
    match published {
        Ok(()) => {
            let mtime = fs::metadata(target).map(|m| mtime_ms(&m)).unwrap_or_else(|_| now_ms());
            Ok(Stamp::of_bytes(bytes, mtime))
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(Error::AlreadyExists(target.to_path_buf()))
        }
        Err(e) => Err(Error::io(target, e)),
    }
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Read a file and stamp it in one pass. `Ok(None)` means it does not exist.
pub fn read_stamped(path: &Path) -> Result<Option<(String, Stamp)>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(path, e)),
    };
    let bytes = fs::read(path).map_err(|e| Error::io(path, e))?;
    let stamp = Stamp::of_bytes(&bytes, mtime_ms(&meta));
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Some((text, stamp)))
}

/// The cheap half of the check: `(mtime, size)` without reading the contents.
pub fn peek(path: &Path) -> Result<Option<(i64, u64)>> {
    match fs::metadata(path) {
        Ok(m) => Ok(Some((mtime_ms(&m), m.len()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

#[derive(Debug)]
pub enum WriteOutcome {
    /// The write landed on the target.
    Written(Stamp),
    /// Someone else changed the target since we loaded it. Our content was
    /// written to `conflict_path` instead; the target is untouched.
    Conflict { conflict_path: PathBuf, stamp: Stamp },
}

/// Write `contents` to `target`, but only if the file still looks the way we
/// last saw it.
///
/// `expected` is the stamp from when we loaded the file, or `None` if we believe
/// the file is new. A mismatch either way is a conflict.
///
/// `peer` is this installation's alias — [`crate::peer::alias`] — and it is what
/// a losing write is stamped with. Taken as an argument rather than read here so
/// that the whole of one session's siblings carry one name; `Project` reads it
/// once at open and passes the same string every time.
pub fn guarded_write(
    project_root: &Path,
    target: &Path,
    contents: &str,
    expected: Option<&Stamp>,
    peer: &str,
) -> Result<WriteOutcome> {
    let bytes = contents.as_bytes();

    let current = match fs::metadata(target) {
        Ok(m) => {
            let existing = fs::read(target).map_err(|e| Error::io(target, e))?;
            Some(Stamp::of_bytes(&existing, mtime_ms(&m)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::io(target, e)),
    };

    let clobbers_someone = match (&current, expected) {
        // We think it is new, but something is already there.
        (Some(_), None) => true,
        // We loaded it; it has changed since. Identical content is not a
        // conflict — two people saving the same bytes is a no-op, not a loss.
        (Some(now), Some(expected)) => now.hash != expected.hash && now.hash != hash_bytes(bytes),
        // Deleted under us, or genuinely new. Recreating is the safe move: the
        // content still exists in the index and nobody loses text.
        (None, _) => false,
    };

    if clobbers_someone {
        let (conflict_path, stamp) = park_conflict(project_root, target, contents, peer)?;
        return Ok(WriteOutcome::Conflict { conflict_path, stamp });
    }

    stage_and_rename(project_root, target, bytes)?;
    let mtime = fs::metadata(target).map(|m| mtime_ms(&m)).unwrap_or_else(|_| now_ms());
    Ok(WriteOutcome::Written(Stamp::of_bytes(bytes, mtime)))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Park `contents` beside `target` as a conflict sibling. **`target` is not
/// read, not compared and not touched.**
///
/// [`guarded_write`]'s losing branch, made reachable on its own. The reason it
/// has to be is [`crate::apply`]: a version arriving from a replicating peer can
/// be a conflict *while the local file is byte-for-byte what we last saw*, which
/// is a state `guarded_write` is structurally unable to represent. Its
/// compare-and-swap answers "has this file moved since I read it", and on a
/// replica the answer is no — both machines wrote successfully, to their own
/// copy, and neither CAS ever failed. Handing that case to `guarded_write` would
/// see an unchanged target, take the write, and overwrite one person's afternoon
/// with another's. So the decision is made a layer up, out of three hashes, and
/// this is the half of the machinery it needs once it has decided.
///
/// `peer` names whoever wrote `contents`, which for an incoming version is the
/// *sender* and not this installation. Stamping it with our own alias would put
/// somebody else's paragraph on a card under our name.
///
/// Nothing here is a shortcut around the guard: the target is untouched in every
/// path, so the worst this can do is create a file. It is deliberately not
/// `pub(crate)` — `wobu-sync` never calls it, but the same "park without
/// comparing" is what any future replication path needs, and hiding it would
/// invite that path to reach for a plain write instead.
pub fn park_conflict(
    project_root: &Path,
    target: &Path,
    contents: &str,
    peer: &str,
) -> Result<(PathBuf, Stamp)> {
    let bytes = contents.as_bytes();
    let conflict_path = reserve_conflict_sibling(target, peer)?;
    stage_and_rename(project_root, &conflict_path, bytes)?;
    Ok((conflict_path, Stamp::of_bytes(bytes, now_ms())))
}

/// `kael-vantris.md` → `kael-vantris.conflict-amber-heron-4f1a-20260731T142211Z.md`
///
/// The Obsidian/Dropbox convention: predictable, sorts next to the original,
/// and because it is Markdown a human — or git — can resolve it properly.
fn conflict_sibling(target: &Path, peer: &str, ts: &str, attempt: u32) -> PathBuf {
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = target.extension().map(|s| s.to_string_lossy().into_owned());
    // A peer alias is already a slug, so this is a no-op on every name Wobu
    // produces. Kept because the argument is a `&str` and a caller could hand
    // over anything — and because a filename with a stray character in it is one
    // an SMB share may refuse, which would lose the very text being rescued.
    let peer = wobu_core::slugify(peer).unwrap_or_else(|_| "unknown".to_string());
    // The first one gets the clean name; only a collision pays for the suffix.
    let n = if attempt <= 1 { String::new() } else { format!("-{attempt}") };
    let name = match ext {
        Some(ext) => format!("{stem}.conflict-{peer}-{ts}{n}.{ext}"),
        None => format!("{stem}.conflict-{peer}-{ts}{n}"),
    };
    target.with_file_name(name)
}

/// Claim a conflict filename that is definitely not in use.
///
/// The timestamp in the name is only accurate to the second, and two conflicts
/// on one file inside the same second are entirely ordinary on a share — a sync
/// client delivering a batch, or two people saving together. Renaming onto a
/// name that already exists would silently destroy the first loser's text,
/// which is the precise failure this whole module exists to prevent: the write
/// that "succeeded" and the version nobody can find afterwards.
///
/// The name is claimed with `create_new`, which is atomic, rather than by
/// testing for existence first. Two Wobu processes on the same share race here
/// for real, and an `exists()` check would let both decide the same name is
/// free. `stage_and_rename` then renames over our own empty placeholder.
fn reserve_conflict_sibling(target: &Path, peer: &str) -> Result<PathBuf> {
    if let Some(parent) = target.parent() {
        crate::paths::ensure_dir(parent)?;
    }
    let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

    for attempt in 1..=MAX_CONFLICT_ATTEMPTS {
        let candidate = conflict_sibling(target, peer, &ts, attempt);
        match fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io(&candidate, e)),
        }
    }
    // A thousand conflicts on one file in one second is not a race, it is a
    // loop somewhere. Failing loudly beats quietly overwriting the thousandth.
    Err(Error::io(target, std::io::Error::other("could not find a free conflict filename")))
}

const MAX_CONFLICT_ATTEMPTS: u32 = 1000;

/// Stage into `.wobu/tmp` — same filesystem as the target, so `rename` is
/// atomic — then rename over the destination.
fn stage_and_rename(project_root: &Path, target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() {
        crate::paths::ensure_dir(parent)?;
    }
    StagedFile::new(project_root, bytes)?.rename(target)
}

/// Replace mutable project metadata without ever exposing partially written
/// bytes. Unix can atomically rename over an existing target. Windows cannot,
/// so it publishes the synced staging inode with hard links and keeps the old
/// inode at `recovery_name` until the new name is in place. [`recover_replace`]
/// restores that old inode if the process dies in the short remove/link gap.
pub fn replace_metadata(
    project_root: &Path,
    target: &Path,
    recovery_name: &str,
    bytes: &[u8],
) -> Result<()> {
    #[cfg(not(windows))]
    let _ = recovery_name;
    let mut staged = StagedFile::new(project_root, bytes)?;

    #[cfg(not(windows))]
    let published = fs::rename(&staged.path, target).map_err(|error| Error::io(target, error));

    #[cfg(windows)]
    let published = {
        let recovery = project_root.join(".wobu").join(recovery_name);
        if target.is_file() && recovery.exists() {
            fs::remove_file(&recovery).map_err(|error| Error::io(&recovery, error))?;
        }
        fs::hard_link(target, &recovery).map_err(|error| Error::io(&recovery, error))?;
        if let Err(error) = fs::remove_file(target) {
            let _ = fs::remove_file(&recovery);
            return Err(Error::io(target, error));
        }
        match fs::hard_link(&staged.path, target) {
            Ok(()) => {
                let _ = fs::remove_file(&recovery);
                Ok(())
            }
            Err(error) => {
                let _ = fs::hard_link(&recovery, target);
                Err(Error::io(target, error))
            }
        }
    };

    #[cfg(not(windows))]
    if published.is_ok() {
        staged.path = PathBuf::new();
    }
    published
}

/// Restore the old metadata inode after an interrupted Windows replacement.
/// A hard link is a no-clobber publication: if another process already restored
/// or replaced the target, this leaves its winner untouched.
pub fn recover_replace(project_root: &Path, target: &Path, recovery_name: &str) -> Result<()> {
    if target.is_file() {
        return Ok(());
    }
    let recovery = project_root.join(".wobu").join(recovery_name);
    if !recovery.is_file() {
        return Ok(());
    }
    match fs::hard_link(&recovery, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(Error::io(target, error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("nodes/character")).unwrap();
        dir
    }

    fn target(root: &Path) -> PathBuf {
        root.join("nodes/character/kael-vantris.md")
    }

    fn exact_existing(path: &Path, expected: &[u8]) -> Result<ExistingContent> {
        let actual = fs::read(path).map_err(|error| Error::io(path, error))?;
        if actual == expected {
            Ok(ExistingContent::Valid)
        } else {
            Err(Error::io(
                path,
                std::io::Error::new(std::io::ErrorKind::InvalidData, "unexpected bytes"),
            ))
        }
    }

    #[test]
    fn content_paths_cannot_escape_the_project() {
        assert!(ProjectRelativePath::new("assets/originals/ab/abc.png").is_ok());
        for invalid in [
            "",
            "/tmp/outside",
            "../outside",
            "assets/../outside",
            "./assets/file",
            "assets//file",
            "assets/file/",
            "assets\\file",
            "C:/outside",
        ] {
            assert!(ProjectRelativePath::new(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn failed_content_batch_is_resumable_and_leaves_no_partial_or_overwrite() {
        let dir = project();
        let first = ProjectRelativePath::new("assets/originals/aa/first.bin").unwrap();
        let second = ProjectRelativePath::new("assets/originals/bb/second.bin").unwrap();
        let first_bytes = b"first winner";
        let second_bytes = b"second winner";

        assert_eq!(
            publish_content_addressed(
                dir.path(),
                &first,
                first_bytes,
                |_| Ok(()),
                |path| exact_existing(path, first_bytes),
            )
            .unwrap(),
            ContentPublish::Published
        );
        let failure = publish_content_addressed_with_link(
            dir.path(),
            &second,
            second_bytes,
            |_| Ok(()),
            |path| exact_existing(path, second_bytes),
            |_, _| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "injected")),
        );
        assert!(failure.is_err());
        assert_eq!(fs::read(first.resolve(dir.path())).unwrap(), first_bytes);
        assert!(!second.resolve(dir.path()).exists());
        assert!(fs::read_dir(dir.path().join(".wobu/tmp")).unwrap().next().is_none());

        assert_eq!(
            publish_content_addressed(
                dir.path(),
                &first,
                first_bytes,
                |_| Ok(()),
                |path| exact_existing(path, first_bytes),
            )
            .unwrap(),
            ContentPublish::Existing
        );
        assert_eq!(
            publish_content_addressed(
                dir.path(),
                &second,
                second_bytes,
                |_| Ok(()),
                |path| exact_existing(path, second_bytes),
            )
            .unwrap(),
            ContentPublish::Published
        );

        let overwrite = publish_content_addressed(
            dir.path(),
            &first,
            b"other content",
            |_| Ok(()),
            |path| exact_existing(path, b"other content"),
        );
        assert!(overwrite.is_err());
        assert_eq!(fs::read(first.resolve(dir.path())).unwrap(), first_bytes);
    }

    #[test]
    fn a_competing_content_winner_closes_the_windows_create_race() {
        let dir = project();
        let relative = ProjectRelativePath::new("assets/originals/aa/race.bin").unwrap();
        let bytes = b"same winner";
        let outcome = publish_content_addressed_with_link(
            dir.path(),
            &relative,
            bytes,
            |_| Ok(()),
            |path| exact_existing(path, bytes),
            |_, target| {
                fs::write(target, bytes)?;
                Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "race winner"))
            },
        )
        .unwrap();
        assert_eq!(outcome, ContentPublish::Existing);
        assert_eq!(fs::read(relative.resolve(dir.path())).unwrap(), bytes);
        assert!(fs::read_dir(dir.path().join(".wobu/tmp")).unwrap().next().is_none());
    }

    #[test]
    fn writes_a_new_file() {
        let dir = project();
        let t = target(dir.path());
        let out = guarded_write(dir.path(), &t, "hello", None, "jake").unwrap();
        assert!(matches!(out, WriteOutcome::Written(_)));
        assert_eq!(fs::read_to_string(&t).unwrap(), "hello");
    }

    #[test]
    fn updates_a_file_we_still_recognise() {
        let dir = project();
        let t = target(dir.path());
        let WriteOutcome::Written(first) =
            guarded_write(dir.path(), &t, "v1", None, "jake").unwrap()
        else {
            panic!("expected a write")
        };

        let out = guarded_write(dir.path(), &t, "v2", Some(&first), "jake").unwrap();
        assert!(matches!(out, WriteOutcome::Written(_)));
        assert_eq!(fs::read_to_string(&t).unwrap(), "v2");
    }

    #[test]
    fn a_changed_target_becomes_a_conflict_sibling_and_never_clobbers() {
        let dir = project();
        let t = target(dir.path());
        let WriteOutcome::Written(loaded) =
            guarded_write(dir.path(), &t, "v1", None, "jake").unwrap()
        else {
            panic!("expected a write")
        };

        // Nadia saves over it while we were editing.
        fs::write(&t, "nadia's version").unwrap();

        let out = guarded_write(dir.path(), &t, "jake's version", Some(&loaded), "jake").unwrap();
        let WriteOutcome::Conflict { conflict_path, .. } = out else {
            panic!("expected a conflict")
        };

        assert_eq!(fs::read_to_string(&t).unwrap(), "nadia's version", "theirs survives");
        assert_eq!(fs::read_to_string(&conflict_path).unwrap(), "jake's version", "ours is kept");
        let name = conflict_path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("kael-vantris.conflict-jake-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
    }

    #[test]
    fn writing_identical_bytes_is_not_a_conflict() {
        // Two people saving the same text loses nothing, so raising a conflict
        // card would just be noise.
        let dir = project();
        let t = target(dir.path());
        let WriteOutcome::Written(loaded) =
            guarded_write(dir.path(), &t, "v1", None, "jake").unwrap()
        else {
            panic!("expected a write")
        };

        fs::write(&t, "same").unwrap();
        let out = guarded_write(dir.path(), &t, "same", Some(&loaded), "jake").unwrap();
        assert!(matches!(out, WriteOutcome::Written(_)));
    }

    #[test]
    fn creating_over_an_existing_file_is_a_conflict() {
        let dir = project();
        let t = target(dir.path());
        fs::write(&t, "someone got here first").unwrap();
        let out = guarded_write(dir.path(), &t, "mine", None, "jake").unwrap();
        assert!(matches!(out, WriteOutcome::Conflict { .. }));
        assert_eq!(fs::read_to_string(&t).unwrap(), "someone got here first");
    }

    #[test]
    fn a_file_deleted_under_us_is_recreated() {
        let dir = project();
        let t = target(dir.path());
        let WriteOutcome::Written(loaded) =
            guarded_write(dir.path(), &t, "v1", None, "jake").unwrap()
        else {
            panic!("expected a write")
        };
        fs::remove_file(&t).unwrap();

        let out = guarded_write(dir.path(), &t, "v2", Some(&loaded), "jake").unwrap();
        assert!(matches!(out, WriteOutcome::Written(_)));
        assert_eq!(fs::read_to_string(&t).unwrap(), "v2");
    }

    #[test]
    fn staging_never_leaves_litter_behind() {
        let dir = project();
        let t = target(dir.path());
        guarded_write(dir.path(), &t, "v1", None, "jake").unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir.path().join(".wobu/tmp")).unwrap().collect();
        assert!(leftovers.is_empty(), "staging dir should be empty after a write");
    }

    #[test]
    fn a_second_conflict_in_the_same_second_does_not_overwrite_the_first() {
        // The conflict filename is stamped to the second, so this collided
        // silently: the second loser's text renamed straight over the first
        // loser's. Two saves landing in one second is ordinary on a share.
        let dir = project();
        let t = target(dir.path());
        let WriteOutcome::Written(loaded) =
            guarded_write(dir.path(), &t, "v1", None, "jake").unwrap()
        else {
            panic!("expected a write")
        };

        fs::write(&t, "theirs").unwrap();
        let WriteOutcome::Conflict { conflict_path: first, .. } =
            guarded_write(dir.path(), &t, "mine one", Some(&loaded), "jake").unwrap()
        else {
            panic!("expected a conflict")
        };
        let WriteOutcome::Conflict { conflict_path: second, .. } =
            guarded_write(dir.path(), &t, "mine two", Some(&loaded), "jake").unwrap()
        else {
            panic!("expected a conflict")
        };

        assert_ne!(first, second, "both conflicts claimed the same filename");
        assert_eq!(fs::read_to_string(&first).unwrap(), "mine one");
        assert_eq!(fs::read_to_string(&second).unwrap(), "mine two");
        assert_eq!(fs::read_to_string(&t).unwrap(), "theirs");
    }

    #[test]
    fn the_first_conflict_of_a_second_still_gets_the_clean_name() {
        // The suffix is a collision cost, not the default shape of the name.
        let dir = project();
        let t = target(dir.path());
        fs::write(&t, "theirs").unwrap();
        let WriteOutcome::Conflict { conflict_path, .. } =
            guarded_write(dir.path(), &t, "mine", None, "jake").unwrap()
        else {
            panic!("expected a conflict")
        };
        let name = conflict_path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("kael-vantris.conflict-jake-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
        // `...-20260731T142211Z.md`, with no `-2` before the extension.
        assert!(!name.contains("-1.md"), "{name}");
    }

    #[test]
    fn parking_a_conflict_leaves_the_target_exactly_as_it_was() {
        // The case `guarded_write` cannot express: a genuine conflict where the
        // file on disk is precisely what we last read. Its CAS would take the
        // write. This must not, ever, under any circumstance, touch the target.
        let dir = project();
        let t = target(dir.path());
        fs::write(&t, "ours, unchanged").unwrap();
        let before = fs::metadata(&t).unwrap();

        let (parked, stamp) =
            park_conflict(dir.path(), &t, "theirs, from another machine", "amber-heron-4f1a")
                .unwrap();

        assert_eq!(fs::read_to_string(&t).unwrap(), "ours, unchanged", "the target was written to");
        assert_eq!(fs::metadata(&t).unwrap().len(), before.len());
        assert_eq!(fs::read_to_string(&parked).unwrap(), "theirs, from another machine");
        assert_eq!(stamp.hash, hash_bytes(b"theirs, from another machine"));
    }

    #[test]
    fn a_parked_conflict_carries_the_name_of_whoever_wrote_it() {
        let dir = project();
        let t = target(dir.path());
        fs::write(&t, "ours").unwrap();
        let (parked, _) = park_conflict(dir.path(), &t, "theirs", "silver-plover-00ff").unwrap();
        let name = parked.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("kael-vantris.conflict-silver-plover-00ff-"), "{name}");
        assert!(name.ends_with(".md"), "{name}");
    }

    #[test]
    fn parking_twice_in_one_second_keeps_both_versions() {
        // A replicating peer delivers a batch, and two versions of one file can
        // land inside the same second the same way two saves can. The sibling
        // name is stamped to the second, so without the reservation the second
        // would rename straight over the first.
        let dir = project();
        let t = target(dir.path());
        fs::write(&t, "ours").unwrap();

        let (first, _) = park_conflict(dir.path(), &t, "theirs one", "amber-heron-4f1a").unwrap();
        let (second, _) = park_conflict(dir.path(), &t, "theirs two", "amber-heron-4f1a").unwrap();

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&first).unwrap(), "theirs one");
        assert_eq!(fs::read_to_string(&second).unwrap(), "theirs two");
        assert_eq!(fs::read_to_string(&t).unwrap(), "ours");
    }

    #[test]
    fn parking_a_conflict_beside_a_file_that_is_not_there_still_keeps_the_text() {
        // The node file deleted locally while a peer's version was in flight.
        // There is nothing to park *beside*, and the incoming paragraph is now
        // the only copy anywhere — dropping it would be the one unrecoverable
        // outcome available here.
        let dir = project();
        let t = target(dir.path());
        let (parked, _) = park_conflict(dir.path(), &t, "theirs", "amber-heron-4f1a").unwrap();
        assert_eq!(fs::read_to_string(&parked).unwrap(), "theirs");
        assert!(!t.exists(), "parking created the target");
    }

    #[test]
    fn stamps_detect_content_change_even_at_identical_size() {
        let a = Stamp::of_bytes(b"aaaa", 0);
        let b = Stamp::of_bytes(b"bbbb", 0);
        assert_eq!(a.size, b.size);
        assert_ne!(a.hash, b.hash, "size alone would have missed this");
    }
}
