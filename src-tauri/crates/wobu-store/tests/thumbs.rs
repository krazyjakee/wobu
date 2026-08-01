//! Thumbnails in the project folder (#25).
//!
//! The claim under test is the one the issue makes and the one `assets` was
//! built on before it: a thumbnail is content-addressed, so it is **conflict-
//! free**, so the first person to import a picture pays for it and everybody
//! else on the share gets it for nothing. A local cache under app data would
//! get the opposite — every collaborator paying separately, for bytes that are
//! identical on all of their machines.
//!
//! Everything here follows from that:
//!
//! - the path is `assets/thumbs/<shard>/<hash>.webp`, sharded exactly like the
//!   original it describes, so one picture cannot become two files;
//! - a thumbnail that is already there is never rewritten, which is what makes
//!   two builds' differing encoders harmless rather than a write war;
//! - and **nothing may assume one exists**, because a folder that arrived over
//!   sync, out of a zip or off a USB stick can hold a full `assets/originals/`
//!   and no `assets/thumbs/` at all.
//!
//! The fixtures are real encoded pictures rather than the hand-written headers
//! the rest of this crate is tested with. That is the point of the file: a
//! header is enough to import and is not enough to decode, and the two failure
//! modes are told apart below.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat, RgbImage, RgbaImage};
use wobu_core::AssetKind;
use wobu_store::thumbs::{self, MAX_SIDE};
use wobu_store::{Cancel, Error, Project, ScanProgress};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A real PNG with real pixel data, which is what separates these fixtures from
/// the header-only ones in `import_limits.rs`: these have to come back out of a
/// decoder. Every size gives different bytes and so a different hash.
fn png(width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x ^ y) % 239) as u8])
    });
    encoded(DynamicImage::ImageRgb8(img))
}

/// The same, with half of it transparent.
fn cutout(width: u32, height: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(width, height, |x, _| {
        image::Rgba([0xf0, 0x30, 0x50, if x < width / 2 { 0 } else { 255 }])
    });
    encoded(DynamicImage::ImageRgba8(img))
}

fn encoded(img: DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}

/// A PNG header and nothing else — enough to import, because `crate::image`
/// reads headers by hand and a half-copied file on a share is ordinary, and
/// nowhere near enough to decode.
fn header_only(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

fn new_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}

/// Import `bytes` and draw its thumbnail, the way the shell's `asset_import`
/// does — two steps, because an import decodes nothing and a thumbnail is
/// nothing but a decode.
fn import_and_thumb(project: &mut Project, bytes: &[u8]) -> (wobu_core::Asset, Option<String>) {
    let imported = project.import_asset(bytes, AssetKind::Reference).unwrap();
    let id = imported.asset.id;
    let thumb = project.ensure_thumb(id, &Cancel::new()).unwrap();
    (imported.asset, thumb)
}

fn thumb_file(project: &Project, hash: &str) -> std::path::PathBuf {
    project.root().join(thumbs::rel_path(hash).replace('/', std::path::MAIN_SEPARATOR_STR))
}

fn size_of(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

/* ── where a thumbnail goes ───────────────────────────────────────────────── */

#[test]
fn a_thumbnail_lands_beside_its_original_under_the_same_sharding() {
    // `assets/thumbs/a3/a3f9…c1.webp`, from the issue. The shard is the first
    // two hex characters of the *original's* hash, which is what keeps one
    // directory small enough for an SMB share to list — and what makes the
    // thumbnail findable from an asset row without a second lookup.
    let (_dir, mut project) = new_project();
    let bytes = png(800, 600);

    let (asset, thumb) = import_and_thumb(&mut project, &bytes);
    let rel = thumb.expect("a picture that decodes must get a thumbnail");

    assert_eq!(rel, format!("assets/thumbs/{}/{}.webp", &asset.hash[..2], asset.hash));
    assert!(thumb_file(&project, &asset.hash).is_file());
    // The same shard as the original, from the same hash — one picture, one
    // directory, on every machine that can see the share.
    assert!(asset.rel_path.starts_with(&format!("assets/originals/{}/", &asset.hash[..2])));
}

#[test]
fn the_index_only_claims_a_thumbnail_once_there_is_one() {
    // `thumbPath: null` is what the grid reads as "draw a placeholder and go
    // and ask". A row claiming a file that is not there is a broken image, so
    // an import — which decodes nothing — must not write one optimistically.
    let (_dir, mut project) = new_project();
    let imported = project.import_asset(&png(300, 300), AssetKind::Reference).unwrap();
    assert_eq!(imported.asset.thumb_path, None);
    assert_eq!(project.list_assets().unwrap()[0].thumb_path, None);

    let rel = project.ensure_thumb(imported.asset.id, &Cancel::new()).unwrap();
    assert_eq!(project.list_assets().unwrap()[0].thumb_path, rel);
    assert!(rel.is_some());
}

#[test]
fn a_thumbnail_is_a_small_webp_and_not_a_copy_of_the_picture() {
    // The entire reason the grid binds to these. A tile that loaded the
    // original would pull megabytes off a share to draw 180 pixels.
    let (_dir, mut project) = new_project();
    let bytes = png(2400, 1800);

    let (asset, _) = import_and_thumb(&mut project, &bytes);
    let path = thumb_file(&project, &asset.hash);
    let drawn = fs::read(&path).unwrap();

    let info = wobu_store::image::probe(&drawn).expect("a thumbnail has to be a readable image");
    assert_eq!(info.mime, "image/webp");
    assert_eq!((info.width, info.height), (MAX_SIDE, MAX_SIDE * 3 / 4));
    assert!(size_of(&path) < asset.bytes / 4, "{} vs {}", size_of(&path), asset.bytes);
}

#[test]
fn a_transparent_reference_is_still_transparent_in_the_grid() {
    // A cut-out PNG flattened to RGB comes back with a black rectangle where
    // the transparency was, which on a dark grid looks like a broken tile
    // rather than like a mistake anyone would go looking for.
    let (_dir, mut project) = new_project();
    let (asset, _) = import_and_thumb(&mut project, &cutout(400, 400));

    let drawn = image::load_from_memory(&fs::read(thumb_file(&project, &asset.hash)).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(drawn.get_pixel(5, 5).0[3], 0, "the transparent half was flattened");
    assert_eq!(drawn.get_pixel(drawn.width() - 5, 5).0[3], 255);
}

/* ── the first importer pays, and nobody pays twice ───────────────────────── */

#[test]
fn the_second_person_to_ask_gets_the_one_the_first_paid_for() {
    // The claim the issue makes, and the reason these files are in the project
    // folder rather than in app data: the work is done once per picture, not
    // once per person. Nothing is rewritten, so nobody's read is ever racing
    // somebody else's write of identical-looking bytes.
    let (_dir, mut project) = new_project();
    let (asset, _) = import_and_thumb(&mut project, &png(900, 900));
    let path = thumb_file(&project, &asset.hash);

    // A sentinel in place of the drawn file: if a second request redraws, it
    // disappears. Deliberately not a byte comparison — two encoders can
    // legitimately differ, and what must not happen is a *write*.
    fs::write(&path, b"RIFF____WEBPdrawn by somebody else").unwrap();
    let again =
        thumbs::ensure(project.root(), &asset.hash, &asset.rel_path, &Cancel::new()).unwrap();

    assert!(!again.generated, "an existing thumbnail was drawn again");
    assert_eq!(fs::read(&path).unwrap(), b"RIFF____WEBPdrawn by somebody else");
}

#[test]
fn two_projects_holding_the_same_picture_agree_on_where_its_thumbnail_goes() {
    // Content addressing all the way down. Jake and Nadia importing the same
    // reference on one share produce one blob and one thumbnail path; if these
    // disagreed, the share would carry two previews of one picture and the
    // conflict-free claim would cover the bytes but not the previews.
    let (_jakes, mut jake) = new_project();
    let (_hers, mut nadia) = new_project();
    let bytes = png(640, 480);

    let (his, his_thumb) = import_and_thumb(&mut jake, &bytes);
    let (hers, her_thumb) = import_and_thumb(&mut nadia, &bytes);

    assert_eq!(his.hash, hers.hash);
    assert_eq!(his_thumb, her_thumb);
    assert!(his_thumb.is_some());
}

/* ── a folder that arrived without any ────────────────────────────────────── */

#[test]
fn a_project_whose_thumbs_directory_never_arrived_draws_them_all_on_demand() {
    // The lazy-regeneration case, stated the way it actually happens: sync,
    // zip and USB all deliver `assets/originals/` and can perfectly well leave
    // `assets/thumbs/` behind. Nothing in Wobu may assume one exists.
    let (_dir, mut project) = new_project();
    for size in [200u32, 300, 400] {
        project.import_asset(&png(size, size), AssetKind::Reference).unwrap();
    }
    for asset in project.list_assets().unwrap() {
        project.ensure_thumb(asset.id, &Cancel::new()).unwrap();
    }

    // The folder as it arrives on the other machine: every blob, no previews.
    fs::remove_dir_all(project.root().join("assets/thumbs")).unwrap();
    project.rebuild_index().unwrap();
    let missing = project.missing_thumbs().unwrap();
    assert_eq!(missing.len(), 3, "a rebuilt index has to notice they are gone");

    let mut seen = Vec::new();
    let present =
        thumbs::ensure_all(project.root(), &missing, &Cancel::new(), &mut |p| seen.push(p))
            .unwrap();
    project.record_thumbs(&present).unwrap();

    assert_eq!(present.len(), 3);
    assert!(project.list_assets().unwrap().iter().all(|a| a.thumb_path.is_some()));
    // And the pass reports itself while it runs, because a stalled mount and a
    // large library look identical from outside.
    assert_eq!(seen.first(), Some(&ScanProgress { done: 0, total: 3 }));
    assert_eq!(seen.last(), Some(&ScanProgress { done: 3, total: 3 }));
}

#[test]
fn a_thumbnail_a_collaborator_drew_is_recorded_rather_than_redrawn() {
    // The reconcile deliberately never re-describes a blob whose path it has
    // already seen — that is what keeps it cheap on a share — so a thumbnail
    // arriving from the far side would otherwise leave our row saying null
    // forever. The bulk pass reports every id that *now has* one, not only the
    // ones it drew itself.
    let (_dir, mut project) = new_project();
    let imported = project.import_asset(&png(500, 500), AssetKind::Reference).unwrap();
    let hash = imported.asset.hash.clone();

    // Their machine's file, landing in our folder with nothing to announce it.
    let path = thumb_file(&project, &hash);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"RIFF____WEBPtheirs").unwrap();

    let targets = project.missing_thumbs().unwrap();
    let present =
        thumbs::ensure_all(project.root(), &targets, &Cancel::new(), &mut |_| {}).unwrap();
    project.record_thumbs(&present).unwrap();

    assert_eq!(present, vec![imported.asset.id]);
    assert_eq!(project.list_assets().unwrap()[0].thumb_path, Some(thumbs::rel_path(&hash)));
    assert_eq!(fs::read(&path).unwrap(), b"RIFF____WEBPtheirs", "theirs was overwritten");
}

/* ── what a blob that will not decode does ────────────────────────────────── */

#[test]
fn a_blob_that_will_not_decode_stays_in_the_library_without_a_preview() {
    // Half a file is the ordinary state of a file on a share. The header
    // parsed, which is how it got imported, and the pixels stop early — so the
    // honest answer is "no thumbnail yet", not a broken one and not an asset
    // dropped from the library.
    let (_dir, mut project) = new_project();
    let (asset, thumb) = import_and_thumb(&mut project, &header_only(400, 400));

    assert_eq!(thumb, None);
    assert!(!thumb_file(&project, &asset.hash).exists(), "a broken thumbnail was written");
    assert_eq!(project.list_assets().unwrap().len(), 1, "the asset was dropped");
    assert_eq!(project.list_assets().unwrap()[0].thumb_path, None);
}

#[test]
fn one_undecodable_blob_does_not_stop_the_rest_of_the_library() {
    // A run that gave up on the first bad file would leave a thousand-image
    // library with no previews because of one — and the bad file is usually a
    // copy still in flight, which is fine a second later.
    let (_dir, mut project) = new_project();
    project.import_asset(&header_only(64, 64), AssetKind::Reference).unwrap();
    for size in [120u32, 130] {
        project.import_asset(&png(size, size), AssetKind::Reference).unwrap();
    }

    let targets = project.missing_thumbs().unwrap();
    let present =
        thumbs::ensure_all(project.root(), &targets, &Cancel::new(), &mut |_| {}).unwrap();

    assert_eq!(present.len(), 2, "the good pictures should still have been drawn");
}

/* ── stopping ─────────────────────────────────────────────────────────────── */

#[test]
fn a_thumbnail_pass_can_be_stopped_and_keeps_what_it_had_already_drawn() {
    // The same token an import and a scan use, for the same reason: a library
    // on a wedged mount is minutes of reading, and without a way out it is an
    // app that has to be killed from a terminal. What is drawn is drawn — the
    // files are at paths only their own pictures can claim, so the next pass
    // finds them for free rather than starting over.
    let (_dir, mut project) = new_project();
    for size in [210u32, 220, 230, 240] {
        project.import_asset(&png(size, size), AssetKind::Reference).unwrap();
    }
    let targets = project.missing_thumbs().unwrap();

    let cancel = Cancel::new();
    let outcome = thumbs::ensure_all(project.root(), &targets, &cancel, &mut |p| {
        if p.done == 2 {
            cancel.cancel();
        }
    });
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");

    let drawn = targets.iter().filter(|t| thumbs::exists(project.root(), &t.hash)).count();
    assert_eq!(drawn, 2, "the pass should have kept what it drew before the cancel");

    // And a second pass finishes the job, paying nothing for the two that are
    // already there.
    let present =
        thumbs::ensure_all(project.root(), &targets, &Cancel::new(), &mut |_| {}).unwrap();
    assert_eq!(present.len(), 4);
}

#[test]
fn a_pass_that_was_already_cancelled_writes_nothing_and_is_not_a_failure() {
    let (_dir, mut project) = new_project();
    project.import_asset(&png(256, 256), AssetKind::Reference).unwrap();
    let targets = project.missing_thumbs().unwrap();

    let cancel = Cancel::new();
    cancel.cancel();
    let outcome = thumbs::ensure_all(project.root(), &targets, &cancel, &mut |_| {});

    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    assert!(!project.root().join("assets/thumbs").join(&targets[0].hash[..2]).exists());
}

#[test]
fn drawing_a_thumbnail_leaves_no_staging_litter_behind() {
    // The same `.wobu/tmp` staging an asset write uses, and the same rule: a
    // `.part` left behind is a file `sweep_staging` has to reason about on
    // every open, and on a synced share it replicates to everyone.
    let (_dir, mut project) = new_project();
    import_and_thumb(&mut project, &png(700, 700));

    let leftovers = fs::read_dir(project.root().join(".wobu/tmp")).unwrap().count();
    assert_eq!(leftovers, 0, "staging should be empty after a thumbnail");
}
