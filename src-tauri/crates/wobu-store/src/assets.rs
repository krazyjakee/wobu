//! Importing blobs into `assets/originals/`.
//!
//! Everything here follows from one property, stated in `docs/02-data-model.md`
//! and `docs/07-file-shares.md`: **an asset write is conflict-free by
//! construction**. Two people importing the same reference on the same share
//! produce identical bytes at an identical path, so no write can ever lose
//! anyone's work and there is nothing for the conflict machinery to arbitrate.
//!
//! Three rules keep that true, and each one breaks it if bent:
//!
//! - **The path is a pure function of the bytes.** The hash names the file and
//!   the *detected* format names the extension. Nothing the user typed, dragged
//!   or renamed reaches the path — `ref.png` and `ref.PNG` holding the same
//!   picture must not become two files.
//! - **The id is a pure function of the hash.** See [`asset_id`].
//! - **A blob that is already there is left alone.** Not as an optimisation:
//!   rewriting a file a collaborator is reading, to replace it with the bytes it
//!   already contains, is pure risk for no gain.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::{Asset, AssetKind, Id};

use crate::atomic;
use crate::error::{Error, Result};
use crate::image;
use crate::paths;

/// Where blobs live, relative to the project root.
pub const ORIGINALS_DIR: &str = "assets/originals";

/// A lowercase hex BLAKE3 digest, as it appears in a filename.
const HASH_LEN: usize = 64;

/// What an import did, as opposed to what it produced.
///
/// `deduped` is the part a caller cannot infer: the asset comes back identical
/// either way, which is the whole point of content addressing, so this is the
/// only thing that says whether anything was written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAsset {
    pub asset: Asset,
    /// True when these bytes were already in the folder and no write happened.
    pub deduped: bool,
}

/// The id an asset with this hash has, on every machine, forever.
///
/// This is deliberately *not* a freshly minted ULID, and that is the one real
/// design decision in asset storage. Two things force it:
///
/// - **The index is disposable.** `docs/02-data-model.md` promises that
///   deleting it loses nothing, because everything can be rebuilt from the
///   folder. Nothing on disk records a minted id — the filename is the hash —
///   so a rebuild would mint new ones and every `AssetLink.asset_id` sitting in
///   somebody's frontmatter would dangle. A derived id survives the rebuild
///   because it was never stored in the first place.
/// - **The share.** Jake and Nadia importing the same reference already produce
///   one file. With minted ids they would produce two records for it, and the
///   conflict-free claim would cover the bytes but not the things that point at
///   them. Deriving the id extends the property to the records.
///
/// The cost, stated plainly: a ULID's leading bits normally encode a
/// timestamp, and here they encode hash bits instead, so asset ids do not sort
/// by creation time. `Asset.created_at` is the field for that. The trade is
/// worth it — an id that sorts nicely and dangles after a rebuild is worse than
/// one that sorts arbitrarily and never does.
///
/// `None` for anything that is not a BLAKE3 digest, which is how a stray file
/// dropped into `assets/originals/` stays out of the index rather than becoming
/// an asset with a nonsense id.
pub fn asset_id(hash: &str) -> Option<Id> {
    if hash.len() != HASH_LEN || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    // The leading half of the digest. 128 bits of BLAKE3 puts a collision
    // somewhere past 2^64 distinct images, which no project folder reaches.
    u128::from_str_radix(&hash[..HASH_LEN / 2], 16).ok().map(Id::from)
}

/// Put `bytes` in the folder, or discover they are already there.
///
/// Refuses anything [`image::probe`] cannot identify. That is a first-class
/// answer rather than a failure of nerve: an asset whose dimensions and mime
/// are unknown cannot be thumbnailed, cannot be routed to a conditioning
/// adapter, and would sit in the library as a file nothing can open.
pub fn import(root: &Path, bytes: &[u8], kind: AssetKind) -> Result<ImportedAsset> {
    let Some(info) = image::probe(bytes) else {
        return Err(Error::NotAnImage);
    };

    let hash = atomic::hash_bytes(bytes);
    let rel = wobu_core::asset::original_path(&hash, info.ext);
    let path = paths::from_rel_string(root, &rel);

    // Size rather than a re-hash. The path already asserts the content, so the
    // only thing left to check is whether the file is *whole* — and the one way
    // a wrong-but-same-length file gets to this path is a BLAKE3 collision. A
    // half-copied file, which is the failure that actually happens on a share,
    // is caught by the length and repaired by the write below.
    let deduped = match fs::metadata(&path) {
        Ok(meta) if meta.len() == bytes.len() as u64 => true,
        Ok(_) => {
            stage_and_rename(root, &path, bytes)?;
            false
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            stage_and_rename(root, &path, bytes)?;
            false
        }
        Err(e) => return Err(Error::io(&path, e)),
    };

    Ok(ImportedAsset { asset: describe(root, &hash, &rel, kind, info, bytes.len() as u64), deduped })
}

/// Every blob in the folder, described well enough to index.
///
/// This is what makes the asset table rebuildable. Files that are not named
/// after a hash, or that no longer parse as an image, are left out rather than
/// reported: they are on disk untouched either way, and an index is not the
/// place to raise an alarm about them.
///
/// Deliberately infallible. A rebuild that refuses because one file in a
/// library of a thousand went strange is a rebuild nobody can run.
pub fn scan(root: &Path) -> Vec<Asset> {
    list_paths(root).into_iter().filter_map(|(_, path)| describe_at(root, &path)).collect()
}

/// Every blob in the folder as `(relative path, absolute path)`, without
/// opening any of them.
///
/// Split out from [`scan`] for the reconcile, which compares this against the
/// index and only pays the header read for paths it has not seen. On a share
/// the listing is the cheap half; opening every blob in a large library on
/// every watcher tick is not.
pub fn list_paths(root: &Path) -> Vec<(String, PathBuf)> {
    let originals = paths::from_rel_string(root, ORIGINALS_DIR);
    walkdir::WalkDir::new(&originals)
        // The shard directory, then the file. Anything deeper was not put there
        // by Wobu.
        .max_depth(2)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(root).ok()?;
            Some((paths::to_rel_string(rel), e.path().to_path_buf()))
        })
        .collect()
}

/// Read one blob back off disk as an [`Asset`], or `None` if it is not one.
pub fn describe_at(root: &Path, path: &Path) -> Option<Asset> {
    let hash = path.file_stem()?.to_str()?.to_owned();
    asset_id(&hash)?;

    let info = image::probe_file(path).ok().flatten()?;
    // A blob whose extension disagrees with its header is not at the path
    // `original_path` would produce for it, so nothing else in Wobu could find
    // it by hash. Indexing it anyway would put a second row on the same id and
    // leave which one wins up to directory order.
    if path.extension()?.to_str()? != info.ext {
        return None;
    }

    let rel = paths::to_rel_string(path.strip_prefix(root).ok()?);
    let bytes = fs::metadata(path).ok()?.len();
    // Rebuilt rows come back as references. The folder records what a blob *is*
    // — its bytes, its shape — but never what it was *for*, and guessing
    // `Generated` for a file that no generation record claims would be a
    // fabrication. Generation records are the eventual source for that.
    Some(describe(root, &hash, &rel, AssetKind::Reference, info, bytes))
}

/// Assemble the record. Every field is read from the folder, so the same file
/// describes identically on every machine that can see the share.
fn describe(
    root: &Path,
    hash: &str,
    rel: &str,
    kind: AssetKind,
    info: image::ImageInfo,
    bytes: u64,
) -> Asset {
    let thumb = wobu_core::asset::thumb_path(hash);
    Asset {
        // Safe: `hash` is either a digest we just computed or one `asset_id`
        // has already accepted off a filename.
        id: asset_id(hash).unwrap_or_default(),
        hash: hash.to_owned(),
        kind,
        rel_path: rel.to_owned(),
        // Present only if something has actually made one. Recording the path a
        // thumbnail *would* have would give the UI a broken image to render.
        thumb_path: paths::from_rel_string(root, &thumb).is_file().then_some(thumb),
        mime: info.mime.to_owned(),
        width: info.width,
        height: info.height,
        bytes,
        created_at: first_written(&paths::from_rel_string(root, rel)),
    }
}

/// When the blob landed, taken from the file rather than from the clock.
///
/// A re-import of something imported last year must not restamp it with today,
/// and a rebuild must not stamp the entire library with the moment of the
/// rebuild. The file's mtime is the only record of this that the folder keeps.
fn first_written(path: &Path) -> DateTime<Utc> {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now())
}

/// Stage into `.wobu/tmp` — the same filesystem as the target, so `rename` is
/// atomic — then rename into place.
///
/// The same shape as the staging in [`crate::atomic`], and pointedly not a call
/// into it: `guarded_write` exists to detect a concurrent writer and park a
/// conflict sibling, and both of those are wrong here. Creating over an
/// existing file is a *conflict* there and is the ordinary, expected case here,
/// so routing an asset through it would litter `assets/originals/` with
/// `.conflict-` copies of files that are byte-for-byte identical.
///
/// What the rename does give us is the two-process case. Two Wobus importing
/// the same reference at the same moment stage to separately-named `.part`
/// files and then rename onto the same target; whichever lands second replaces
/// the first with identical bytes, and no reader ever observes a partial file,
/// because a partially written file is never at the target path at all.
fn stage_and_rename(root: &Path, target: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_dir = root.join(".wobu").join("tmp");
    paths::ensure_dir(&tmp_dir)?;
    if let Some(parent) = target.parent() {
        paths::ensure_dir(parent)?;
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

    /// A PNG header and nothing else. Every dimension gives different bytes and
    /// so a different hash, which is all these tests need.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    #[test]
    fn the_id_is_the_hash_and_survives_a_string_round_trip() {
        // The id crosses the bridge as a 26-character ULID string and comes
        // back through `Id::from_string` on every save. A derived id that did
        // not round-trip would break every link the moment it was written into
        // frontmatter.
        let hash = atomic::hash_bytes(b"some bytes");
        let id = asset_id(&hash).unwrap();
        assert_eq!(id, Id::from_string(&id.to_string()).unwrap());
        assert_eq!(asset_id(&hash), Some(id), "the same hash must give the same id");
    }

    #[test]
    fn only_a_blake3_digest_yields_an_id() {
        // Anything else in `assets/originals/` — a `.DS_Store`, a sync client's
        // scratch file — must stay out of the index rather than arrive as an
        // asset with an id derived from its name.
        assert!(asset_id("").is_none());
        assert!(asset_id("not-a-hash").is_none());
        assert!(asset_id(&"a".repeat(63)).is_none());
        assert!(asset_id(&"a".repeat(65)).is_none());
        assert!(asset_id(&"g".repeat(64)).is_none(), "g is not hex");
        // Uppercase is refused as well: `to_hex` is lowercase, so an uppercase
        // digest is a filename something else wrote and its path is not one
        // `original_path` would produce.
        assert!(asset_id(&"A".repeat(64)).is_none());
    }

    #[test]
    fn the_extension_comes_from_the_bytes_and_never_from_a_filename() {
        // The property the whole module rests on. Nothing in this signature
        // even accepts a filename, and that is the point: the same picture
        // imported as `ref.png` and `ref.PNG` has to reach one path.
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(4, 4);
        let out = import(dir.path(), &bytes, AssetKind::Reference).unwrap();
        assert!(out.asset.rel_path.ends_with(".png"), "{}", out.asset.rel_path);
        assert_eq!(out.asset.mime, "image/png");
    }

    #[test]
    fn a_blob_lands_at_the_documented_sharded_path() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(640, 480);
        let out = import(dir.path(), &bytes, AssetKind::Reference).unwrap();

        let hash = atomic::hash_bytes(&bytes);
        assert_eq!(out.asset.rel_path, format!("assets/originals/{}/{hash}.png", &hash[..2]));
        assert!(dir.path().join(&out.asset.rel_path).is_file());
        assert_eq!(out.asset.width, 640);
        assert_eq!(out.asset.height, 480);
        assert_eq!(out.asset.bytes, bytes.len() as u64);
        assert!(!out.deduped);
    }

    #[test]
    fn bytes_that_are_not_a_recognised_image_are_refused_by_name() {
        // A first-class outcome, not an io error and not a panic: the UI has to
        // be able to say "that is not an image" rather than "io error".
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            import(dir.path(), b"this is a text file", AssetKind::Reference),
            Err(Error::NotAnImage)
        ));
        assert!(!dir.path().join("assets").exists(), "nothing should have been written");
    }

    #[test]
    fn a_half_copied_blob_is_repaired_rather_than_trusted() {
        // A sync client copying a file in is the expected way a blob at the
        // right path ends up with the wrong bytes. Treating a short file as a
        // dedup hit would leave the truncated version there forever.
        let dir = tempfile::tempdir().unwrap();
        let bytes = png(16, 16);
        let out = import(dir.path(), &bytes, AssetKind::Reference).unwrap();
        let path = dir.path().join(&out.asset.rel_path);
        fs::write(&path, &bytes[..8]).unwrap();

        let again = import(dir.path(), &bytes, AssetKind::Reference).unwrap();
        assert!(!again.deduped);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn staging_never_leaves_litter_behind() {
        let dir = tempfile::tempdir().unwrap();
        import(dir.path(), &png(2, 2), AssetKind::Reference).unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir.path().join(".wobu/tmp")).unwrap().collect();
        assert!(leftovers.is_empty(), "staging dir should be empty after an import");
    }

    #[test]
    fn a_scan_rebuilds_the_same_records_the_import_returned() {
        // The index is disposable, so what `scan` produces has to be what the
        // import produced — the id above all, because links point at it.
        let dir = tempfile::tempdir().unwrap();
        let imported = import(dir.path(), &png(320, 200), AssetKind::Reference).unwrap();

        let scanned = scan(dir.path());
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0], imported.asset);
    }

    #[test]
    fn a_scan_walks_past_files_it_did_not_write() {
        let dir = tempfile::tempdir().unwrap();
        import(dir.path(), &png(8, 8), AssetKind::Reference).unwrap();
        let shard = dir.path().join(ORIGINALS_DIR).join("aa");
        fs::create_dir_all(&shard).unwrap();
        fs::write(shard.join(".DS_Store"), b"junk").unwrap();
        fs::write(shard.join(format!("{}.png", "b".repeat(64))), b"not really a png").unwrap();

        assert_eq!(scan(dir.path()).len(), 1);
    }
}
