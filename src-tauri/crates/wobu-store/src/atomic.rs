//! Guarded atomic writes.
//!
//! Node Markdown is the only file in a project that two people can edit at
//! once, so it is the only thing this module protects. Every write stages into
//! `.wobu/tmp` on the **same filesystem** — using the OS temp dir would
//! silently degrade the rename into a cross-device copy, which is not atomic —
//! checks that the target has not moved under us, and then renames into place.
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
        Stamp {
            mtime_ms,
            size: bytes.len() as u64,
            hash: blake3::hash(bytes).to_hex().to_string(),
        }
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
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
        let conflict_path = reserve_conflict_sibling(target, peer)?;
        stage_and_rename(project_root, &conflict_path, bytes)?;
        let stamp = Stamp::of_bytes(bytes, now_ms());
        return Ok(WriteOutcome::Conflict { conflict_path, stamp });
    }

    stage_and_rename(project_root, target, bytes)?;
    let mtime = fs::metadata(target).map(|m| mtime_ms(&m)).unwrap_or_else(|_| now_ms());
    Ok(WriteOutcome::Written(Stamp::of_bytes(bytes, mtime)))
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
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
    Err(Error::io(
        target,
        std::io::Error::other("could not find a free conflict filename"),
    ))
}

const MAX_CONFLICT_ATTEMPTS: u32 = 1000;

/// Stage into `.wobu/tmp` — same filesystem as the target, so `rename` is
/// atomic — then rename over the destination.
fn stage_and_rename(project_root: &Path, target: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_dir = project_root.join(".wobu").join("tmp");
    crate::paths::ensure_dir(&tmp_dir)?;
    if let Some(parent) = target.parent() {
        crate::paths::ensure_dir(parent)?;
    }

    let tmp = tmp_dir.join(format!("{}.part", wobu_core::new_id()));
    {
        let mut f = fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| Error::io(&tmp, e))?;
        // Without this, a crash between rename and flush can leave a
        // correctly-named file full of zeroes.
        f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }

    if let Err(e) = fs::rename(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(target, e));
    }
    Ok(())
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
    fn stamps_detect_content_change_even_at_identical_size() {
        let a = Stamp::of_bytes(b"aaaa", 0);
        let b = Stamp::of_bytes(b"bbbb", 0);
        assert_eq!(a.size, b.size);
        assert_ne!(a.hash, b.hash, "size alone would have missed this");
    }
}
