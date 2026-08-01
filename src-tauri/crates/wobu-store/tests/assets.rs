//! What content addressing has to buy, proved from outside the crate.
//!
//! The unit tests in `assets.rs` cover the parsing and the paths. This file
//! covers the promise those exist to keep, from `docs/02-data-model.md`: an
//! asset write is conflict-free by construction, so two people on one share can
//! import the same reference at the same moment and nothing anywhere is lost or
//! duplicated.
//!
//! Four things have to hold for that to be true, and each has a test below:
//!
//! - the path depends on the bytes and on nothing else;
//! - the id depends on the hash and on nothing else, so it is the same on both
//!   machines and survives the index being thrown away;
//! - a blob that is already there is not written again;
//! - two writers racing for the same path cannot produce a partial file.

use std::fs;
use std::path::Path;

use wobu_core::{AssetKind, AssetRole, NodeKind};
use wobu_store::{Error, Project, SaveOutcome};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A PNG header. Only the IHDR chunk is read, and varying the dimensions
/// varies the bytes, which is all these tests need of an image.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

/// SOI, an APP0 block to walk past, then SOF0.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[0; 9]);
    out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    out
}

fn new_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}

fn mtime(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Backdate a file so that any rewrite of it is unmistakable. Filesystem
/// timestamps are coarse enough that two writes in the same millisecond look
/// identical, and this test would otherwise pass without proving anything.
fn backdate(path: &Path, by: std::time::Duration) {
    let when = std::time::SystemTime::now() - by;
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_times(fs::FileTimes::new().set_modified(when)).unwrap();
}

/* ── the path depends on the bytes ────────────────────────────────────────── */

#[test]
fn the_filename_comes_from_the_content_and_never_from_what_was_dropped() {
    // The trap this whole design is built around. Take the extension from the
    // imported filename and the same picture arriving as `ref.png` and
    // `REF.PNG` lands at two paths, at which point two people importing one
    // image no longer produce one file and the conflict-free claim is gone.
    let (_dir, mut project) = new_project();
    let source = tempfile::tempdir().unwrap();

    let bytes = png(200, 100);
    let lower = source.path().join("ref.png");
    let shouty = source.path().join("REF.PNG");
    let liar = source.path().join("photo.jpeg");
    for path in [&lower, &shouty, &liar] {
        fs::write(path, &bytes).unwrap();
    }

    let first = project.import_asset_file(&lower, AssetKind::Reference).unwrap();
    for path in [&shouty, &liar] {
        let again = project.import_asset_file(path, AssetKind::Reference).unwrap();
        assert_eq!(
            again.asset.rel_path,
            first.asset.rel_path,
            "{} took its own path",
            path.display()
        );
        assert_eq!(again.asset.id, first.asset.id);
        assert!(again.deduped);
    }

    assert!(first.asset.rel_path.ends_with(".png"));
    assert_eq!(project.list_assets().unwrap().len(), 1, "one picture, one asset");
}

#[test]
fn a_blob_is_sharded_two_levels_and_named_only_by_its_hash() {
    let (dir, mut project) = new_project();
    let imported = project.import_asset(&png(640, 480), AssetKind::Reference).unwrap();
    let asset = imported.asset;

    assert_eq!(asset.rel_path, format!("assets/originals/{}/{}.png", &asset.hash[..2], asset.hash));
    assert!(dir.path().join("ashfall.wobu").join(&asset.rel_path).is_file());
    assert_eq!(asset.mime, "image/png");
    assert_eq!((asset.width, asset.height), (640, 480));
}

/* ── the id depends on the hash ───────────────────────────────────────────── */

#[test]
fn two_people_importing_one_picture_agree_on_its_id() {
    // The half of "conflict-free" that a minted ULID would quietly drop. The
    // bytes would still land at one path, but Jake's frontmatter and Nadia's
    // would point at two different asset ids for the same file, and neither
    // machine could tell they meant the same thing.
    let bytes = jpeg(1024, 768);
    let (_jake_dir, mut jake) = new_project();
    let (_nadia_dir, mut nadia) = new_project();

    let hers = nadia.import_asset(&bytes, AssetKind::Reference).unwrap().asset;
    let his = jake.import_asset(&bytes, AssetKind::Reference).unwrap().asset;

    assert_eq!(his.id, hers.id);
    assert_eq!(his.hash, hers.hash);
    assert_eq!(his.rel_path, hers.rel_path);
}

#[test]
fn an_asset_id_survives_the_index_being_deleted() {
    // `docs/02-data-model.md` promises the index is disposable. A minted asset
    // id would not be in the folder anywhere, so a rebuild would issue new ones
    // and every `AssetLink` in the world would point at nothing.
    let dir = tempfile::tempdir().unwrap();
    let (root, before) = {
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        let asset = project.import_asset(&png(320, 200), AssetKind::Reference).unwrap().asset;
        fs::remove_file(project.index_path()).ok();
        (project.root().to_path_buf(), asset)
    };

    let reopened = Project::open(&root).unwrap();
    let after = reopened.list_assets().unwrap();
    assert_eq!(after.len(), 1, "the library did not come back");
    assert_eq!(after[0].id, before.id, "the id changed, so every link to it would dangle");
    assert_eq!(after[0].hash, before.hash);
    assert_eq!(after[0].rel_path, before.rel_path);
    assert_eq!((after[0].width, after[0].height), (320, 200));
    assert_eq!(reopened.get_asset(before.id).unwrap().unwrap().mime, "image/png");
}

#[test]
fn a_rebuilt_index_rediscovers_the_whole_library() {
    let (_dir, mut project) = new_project();
    for size in 1..=5u32 {
        project.import_asset(&png(size * 10, size * 10), AssetKind::Reference).unwrap();
    }
    let before = project.list_assets().unwrap();

    project.rebuild_index().unwrap();

    let after = project.list_assets().unwrap();
    assert_eq!(after.len(), 5);
    assert_eq!(
        after.iter().map(|a| a.id).collect::<Vec<_>>(),
        before.iter().map(|a| a.id).collect::<Vec<_>>(),
    );
}

/* ── dedup does not rewrite ───────────────────────────────────────────────── */

#[test]
fn re_importing_a_picture_returns_the_same_asset_and_leaves_the_file_alone() {
    // "Returns the same id" is the cheap half and would pass even if the file
    // were rewritten every time. The file itself is what matters: rewriting a
    // blob a collaborator on the share may be reading, in order to replace it
    // with the bytes it already holds, is risk taken for nothing.
    let (dir, mut project) = new_project();
    let bytes = png(48, 48);

    let first = project.import_asset(&bytes, AssetKind::Reference).unwrap();
    assert!(!first.deduped, "the first import has to actually write");
    let path = dir.path().join("ashfall.wobu").join(&first.asset.rel_path);

    backdate(&path, std::time::Duration::from_secs(3600));
    let stamped = mtime(&path);

    let second = project.import_asset(&bytes, AssetKind::Reference).unwrap();

    assert!(second.deduped);
    assert_eq!(second.asset.id, first.asset.id);
    assert_eq!(mtime(&path), stamped, "the blob was rewritten");
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(project.list_assets().unwrap().len(), 1);
}

#[test]
fn a_dedup_keeps_the_moment_the_picture_first_arrived() {
    // Re-importing something from last year must not restamp it with today, or
    // a library sorted by date reshuffles itself every time somebody drags an
    // old reference back in.
    let (dir, mut project) = new_project();
    let bytes = png(72, 72);
    let first = project.import_asset(&bytes, AssetKind::Reference).unwrap();
    let path = dir.path().join("ashfall.wobu").join(&first.asset.rel_path);

    backdate(&path, std::time::Duration::from_secs(60 * 60 * 24 * 30));
    let second = project.import_asset(&bytes, AssetKind::Reference).unwrap();

    let month = chrono::Utc::now() - chrono::Duration::days(29);
    assert!(second.asset.created_at < month, "{}", second.asset.created_at);
}

/* ── deliberate orphan deletion ─────────────────────────────────────────── */

#[test]
fn deleting_an_orphan_removes_its_blob_and_index_row() {
    let (_dir, mut project) = new_project();
    let asset = project.import_asset(&png(96, 64), AssetKind::Upload).unwrap().asset;
    let original = project.root().join(&asset.rel_path);

    project.delete_asset(asset.id).unwrap();

    assert!(!original.exists());
    assert!(project.get_asset(asset.id).unwrap().is_none());
    assert!(project.list_assets().unwrap().is_empty());
}

#[test]
fn deleting_refuses_links_and_covers_even_after_the_ui_called_an_asset_an_orphan() {
    let (_dir, mut project) = new_project();
    let linked = project.import_asset(&png(100, 80), AssetKind::Reference).unwrap().asset;
    let covered = project.import_asset(&png(101, 80), AssetKind::Upload).unwrap().asset;
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    project.link_asset(node.id, linked.id, AssetRole::FullRef, None).unwrap();
    project.set_cover_asset(node.id, Some(covered.id)).unwrap();

    for asset in [linked, covered] {
        let error = project.delete_asset(asset.id).unwrap_err();
        assert!(matches!(error, Error::AssetInUse { nodes: 1, .. }), "{error}");
        assert!(project.root().join(&asset.rel_path).is_file());
        assert!(project.get_asset(asset.id).unwrap().is_some());
    }
}

#[test]
fn deletion_reads_canonical_markdown_instead_of_trusting_a_stale_local_index() {
    let (_dir, mut project) = new_project();
    let asset = project.import_asset(&png(111, 81), AssetKind::Reference).unwrap().asset;
    let mut node = project.create_node(NodeKind::Character, "Kael", None).unwrap();

    // Simulate Obsidian or a collaborator writing after this Wobu's last
    // reconcile. The local index still says orphan; only the file says linked.
    node.asset_links.push(wobu_core::asset::AssetRef::new(asset.id, AssetRole::FullRef));
    let path = project.root().join("nodes/character/kael.md");
    fs::write(&path, wobu_store::markdown::to_markdown(&node).unwrap()).unwrap();
    assert!(project.index().asset_backlinks(asset.id).unwrap().is_empty());

    let error = project.delete_asset(asset.id).unwrap_err();
    assert!(matches!(error, Error::AssetInUse { nodes: 1, .. }), "{error}");
    assert!(project.root().join(&asset.rel_path).is_file());
}

#[test]
fn asset_usage_groups_roles_cover_and_linked_node_tags() {
    let (_dir, mut project) = new_project();
    let asset = project.import_asset(&png(120, 90), AssetKind::Reference).unwrap().asset;
    let mut node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    node.tags = vec!["playable".into(), "ashlands".into()];
    node.cover_asset_id = Some(asset.id);
    node.asset_links.push(wobu_core::asset::AssetRef::new(asset.id, AssetRole::Palette));
    match project.save_node(node).unwrap() {
        SaveOutcome::Saved(_) => {}
        SaveOutcome::Conflict { conflict_path } => panic!("unexpected conflict at {conflict_path}"),
    }

    let usage = project.asset_usages().unwrap().pop().unwrap();
    assert_eq!(usage.asset_id, asset.id);
    assert_eq!(usage.node_name, "Vashk");
    assert_eq!(usage.node_tags, ["playable", "ashlands"]);
    assert!(usage.cover);
    assert_eq!(usage.roles.len(), 1);
    assert_eq!(usage.roles[0].role, AssetRole::Palette);
}

/* ── two writers ──────────────────────────────────────────────────────────── */

#[test]
fn importing_the_same_bytes_from_several_writers_at_once_cannot_corrupt_the_blob() {
    // Two Wobus on one share, both told to import the same reference. They
    // stage under separate names and rename onto the same target, so the loser
    // replaces the winner's file with bytes identical to it — and a reader
    // opening the path at any instant during all of that sees either nothing or
    // the whole file, never half of one. A bare `fs::write` would let them
    // interleave into a blob that is neither.
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    let root = project.root().to_path_buf();
    let bytes = png(1920, 1080);

    let outcomes: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                let bytes = bytes.clone();
                scope.spawn(move || {
                    wobu_store::assets::import(&root, &bytes, AssetKind::Reference).unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let rel = &outcomes[0].asset.rel_path;
    for outcome in &outcomes {
        assert_eq!(&outcome.asset.rel_path, rel, "the writers disagreed about the path");
        assert_eq!(outcome.asset.id, outcomes[0].asset.id);
    }
    assert_eq!(fs::read(root.join(rel)).unwrap(), bytes, "the blob is not intact");
    assert_eq!(wobu_store::assets::scan(&root).len(), 1, "one picture, one file");

    // Every stage either renamed or cleaned up after itself.
    let litter: Vec<_> = fs::read_dir(root.join(".wobu/tmp")).unwrap().collect();
    assert!(litter.is_empty(), "staging litter left behind");
}

/* ── the share ────────────────────────────────────────────────────────────── */

#[test]
fn a_reference_imported_on_another_machine_appears_on_the_next_reconcile() {
    // Nothing on this side fires when a collaborator writes into the share, so
    // the folder listing is the only signal there will ever be.
    let (_dir, mut project) = new_project();
    let root = project.root().to_path_buf();
    assert!(project.list_assets().unwrap().is_empty());

    let theirs = wobu_store::assets::import(&root, &jpeg(800, 600), AssetKind::Reference).unwrap();

    assert!(project.reconcile().unwrap(), "the import should count as a change");
    let listed = project.list_assets().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, theirs.asset.id);
}

#[test]
fn a_blob_deleted_from_the_folder_leaves_the_index() {
    let (_dir, mut project) = new_project();
    let asset = project.import_asset(&png(64, 64), AssetKind::Reference).unwrap().asset;
    fs::remove_file(project.root().join(&asset.rel_path)).unwrap();

    assert!(project.reconcile().unwrap());
    assert!(project.list_assets().unwrap().is_empty());
    assert!(project.get_asset(asset.id).unwrap().is_none());
}

#[test]
fn reconciling_an_unchanged_library_reports_nothing() {
    // The watcher calls this on every tick. A reconcile that always claimed a
    // change would refetch the whole world in a loop.
    let (_dir, mut project) = new_project();
    project.import_asset(&png(64, 64), AssetKind::Reference).unwrap();
    project.reconcile().unwrap();
    assert!(!project.reconcile().unwrap());
}

/* ── refusals ─────────────────────────────────────────────────────────────── */

#[test]
fn something_that_is_not_an_image_is_refused_and_nothing_is_written() {
    let (_dir, mut project) = new_project();
    let before = wobu_store::assets::scan(project.root()).len();

    let outcome = project.import_asset(b"%PDF-1.7\nnot a reference image", AssetKind::Reference);
    assert!(matches!(outcome, Err(wobu_store::Error::NotAnImage)), "{outcome:?}");

    assert_eq!(wobu_store::assets::scan(project.root()).len(), before);
    assert!(project.list_assets().unwrap().is_empty());
}

#[test]
fn an_asset_record_names_nothing_absolute() {
    // The same share is /Volumes/art/… here and Z:\art\… on the next desk, so
    // an absolute path in a record is a path that is wrong somewhere.
    let (dir, mut project) = new_project();
    let asset = project.import_asset(&png(128, 128), AssetKind::Reference).unwrap().asset;
    let root = dir.path().to_string_lossy().into_owned();

    assert!(!asset.rel_path.contains(&root), "{}", asset.rel_path);
    assert!(!asset.rel_path.starts_with('/'));
    assert!(!asset.rel_path.contains('\\'));
    assert!(asset.thumb_path.is_none(), "nothing has made a thumbnail yet");
}
