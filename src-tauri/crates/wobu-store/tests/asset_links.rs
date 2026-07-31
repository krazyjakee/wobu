//! Attaching reference images to nodes, proved from outside the crate.
//!
//! The interesting part of an asset link is not that it is stored — it is that
//! the **role** decides where the image is routed when a generation is
//! compiled. `docs/04-influence-engine.md` fixes the target vocabulary, and one
//! of those targets, `moodboard_only`, means "never leaves this machine". So
//! the tests here are about four promises:
//!
//! - a `mood` reference cannot reach a backend, whatever else is true of it;
//! - a link to an asset the project does not have is refused, not stored —
//!   #24 went to some trouble to make asset ids survive an index rebuild, and
//!   writing an id that matches no file would undo that from the other end;
//! - the links are in Markdown, so throwing the index away loses nothing;
//! - unlinking never deletes the blob, because assets are shared.

use std::fs;

use wobu_core::asset::AssetRef;
use wobu_core::{AssetKind, AssetRole, FragmentTarget, Id, Node, NodeKind};
use wobu_store::{Error, Project, SaveOutcome};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A PNG header, and nothing more than the parser reads. Different dimensions
/// give different bytes, and therefore a different hash and a different id.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

/// A project with a character and two imported references.
fn world() -> (tempfile::TempDir, Project, Node, Id, Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let a = project.import_asset(&png(640, 480), AssetKind::Reference).unwrap().asset.id;
    let b = project.import_asset(&png(320, 200), AssetKind::Reference).unwrap().asset.id;
    (dir, project, node, a, b)
}

fn saved(outcome: SaveOutcome) -> Node {
    match outcome {
        SaveOutcome::Saved(node) => *node,
        SaveOutcome::Conflict { conflict_path } => panic!("unexpected conflict at {conflict_path}"),
    }
}

/* ── the one that matters ─────────────────────────────────────────────────── */

#[test]
fn a_mood_reference_can_never_be_routed_to_a_backend() {
    // The failure this guards is not a wrong picture — it is a private
    // reference on a third party's servers, which is unrecoverable and which
    // the user would never be told about. Nothing about how a mood link is
    // stored, weighted or enabled may make it sendable.
    let (_dir, mut project, node, asset, _other) = world();

    let node = saved(project.link_asset(node.id, asset, AssetRole::Mood, Some(1.0)).unwrap());
    let mood = &node.asset_links[0];

    assert_eq!(mood.role.target(), FragmentTarget::MoodboardOnly);
    assert!(!mood.is_conditioning(), "a mood link is never conditioning");
    assert!(!mood.role.is_conditioning());

    // Full weight and enabled — the two things a caller might read as consent —
    // change nothing.
    assert_eq!(mood.weight, 1.0);
    assert!(mood.enabled);

    // And the same link read back off disk, since frontmatter is what a compile
    // six months from now will be working from.
    let reread = project.get_node(node.id).unwrap();
    assert!(!reread.asset_links[0].is_conditioning());
    assert_eq!(reread.asset_links[0].role.target(), FragmentTarget::MoodboardOnly);

    // Everything a compiler would send is what is left after the filter, and
    // the mood link is not in it.
    let sendable: Vec<_> = reread.asset_links.iter().filter(|l| l.is_conditioning()).collect();
    assert!(sendable.is_empty(), "{sendable:?} would have been uploaded");
}

#[test]
fn every_role_a_node_can_carry_routes_somewhere_deliberate() {
    // Attaching one of each and reading the roles back off disk, because the
    // mapping is only worth anything if it survives serialisation — a role that
    // round-trips to the wrong string would route to the wrong adapter in
    // silence.
    let (_dir, mut project, node, asset, _other) = world();
    for role in AssetRole::ALL {
        project.link_asset(node.id, asset, role, None).unwrap();
    }

    let reread = project.get_node(node.id).unwrap();
    let roles: Vec<AssetRole> = reread.asset_links.iter().map(|l| l.role).collect();
    assert_eq!(roles, AssetRole::ALL, "every role has to come back as the role it went in as");

    for link in &reread.asset_links {
        assert_eq!(
            link.is_conditioning(),
            link.role != AssetRole::Mood,
            "{} is on the wrong side of the moodboard filter",
            link.role
        );
    }
}

/* ── dangling ids ─────────────────────────────────────────────────────────── */

#[test]
fn linking_an_asset_that_does_not_exist_is_refused_rather_than_stored() {
    // An asset id is derived from a file's hash, so an id matching no file
    // matches no file on any machine, ever. Stored, it would be a permanent
    // dangling reference sitting in frontmatter on a share.
    let (_dir, mut project, node, _asset, _other) = world();
    let ghost = wobu_core::new_id();

    let err = project.link_asset(node.id, ghost, AssetRole::Pose, None).unwrap_err();
    assert!(matches!(&err, Error::NoSuchAsset(id) if *id == ghost.to_string()), "{err:?}");

    // Nothing was written — not the link, and not a half-edit of the node.
    assert!(project.get_node(node.id).unwrap().asset_links.is_empty());
    let cover = project.set_cover_asset(node.id, Some(ghost)).unwrap_err();
    assert!(matches!(cover, Error::NoSuchAsset(_)), "{cover:?}");
    assert_eq!(project.get_node(node.id).unwrap().cover_asset_id, None);
}

#[test]
fn updating_or_removing_a_link_that_is_not_there_says_so() {
    // The stale-UI case, and on a share the collaborator case: somebody removed
    // this reference while the panel was open. Recreating it to satisfy the
    // request would undo their edit.
    let (_dir, mut project, node, asset, _other) = world();

    let err = project.unlink_asset(node.id, asset, AssetRole::Pose).unwrap_err();
    assert!(matches!(err, Error::NoSuchAssetLink { .. }), "{err:?}");

    project.link_asset(node.id, asset, AssetRole::Pose, None).unwrap();
    // Right asset, wrong role: still not a link that exists.
    let err = project.update_asset_link(node.id, asset, AssetRole::Palette, Some(0.5), None);
    assert!(matches!(err.unwrap_err(), Error::NoSuchAssetLink { .. }));
}

/* ── weights ──────────────────────────────────────────────────────────────── */

#[test]
fn asset_weights_default_and_clamp_exactly_as_influence_weights_do() {
    // Two kinds of edge feed one compiler. If an asset link defaulted to
    // something other than 1.0, or clamped to a different range, the same
    // number would mean two things depending on which edge carried it.
    let (_dir, mut project, node, asset, _other) = world();

    let node = saved(project.link_asset(node.id, asset, AssetRole::Palette, None).unwrap());
    assert_eq!(node.asset_links[0].weight, 1.0);
    assert!(node.asset_links[0].enabled);

    let node = saved(
        project.update_asset_link(node.id, asset, AssetRole::Palette, Some(9.0), None).unwrap(),
    );
    assert_eq!(node.asset_links[0].weight, 1.0, "out of range is clamped, not stored");

    let node = saved(
        project.update_asset_link(node.id, asset, AssetRole::Palette, Some(-1.0), None).unwrap(),
    );
    assert_eq!(node.asset_links[0].weight, 0.0);

    // Muting is a separate control and must not disturb the weight.
    let node = saved(
        project.update_asset_link(node.id, asset, AssetRole::Palette, None, Some(false)).unwrap(),
    );
    assert!(!node.asset_links[0].enabled);
    assert_eq!(node.asset_links[0].weight, 0.0);
}

#[test]
fn linking_the_same_asset_in_the_same_role_twice_replaces_rather_than_duplicates() {
    // Two identical rows would be indistinguishable in the UI, so the user
    // could not remove one of them. The same asset in a *different* role is a
    // different link, because it reaches a different adapter.
    let (_dir, mut project, node, asset, _other) = world();

    project.link_asset(node.id, asset, AssetRole::FullRef, Some(0.5)).unwrap();
    let node = saved(project.link_asset(node.id, asset, AssetRole::FullRef, Some(0.8)).unwrap());
    assert_eq!(node.asset_links.len(), 1);
    assert_eq!(node.asset_links[0].weight, 0.8);

    let node = saved(project.link_asset(node.id, asset, AssetRole::Palette, None).unwrap());
    assert_eq!(node.asset_links.len(), 2);
}

/* ── the folder is canonical ──────────────────────────────────────────────── */

#[test]
fn deleting_the_index_loses_no_links_and_no_roles() {
    // The promise in `docs/02-data-model.md`, applied to this table. Asset ids
    // are derived from hashes precisely so that a rebuild does not orphan the
    // links pointing at them; that only pays off if the links themselves are in
    // the Markdown too.
    let (_dir, mut project, node, asset, other) = world();
    project.link_asset(node.id, asset, AssetRole::Pose, Some(0.4)).unwrap();
    project.link_asset(node.id, other, AssetRole::Mood, None).unwrap();
    project.set_cover_asset(node.id, Some(other)).unwrap();

    let root = project.root().to_path_buf();
    let id = project.id();
    drop(project);
    fs::remove_file(wobu_store::paths::index_path(&id)).unwrap();

    let reopened = Project::open(&root).unwrap();
    let reread = reopened.get_node(node.id).unwrap();
    assert_eq!(reread.asset_links.len(), 2);
    assert_eq!(reread.cover_asset_id, Some(other));

    let poses = reopened.index().asset_links_in_role(node.id, AssetRole::Pose).unwrap();
    assert_eq!(poses.len(), 1, "the table was rebuilt from the node file");
    assert_eq!(poses[0].asset_id, asset);
    assert_eq!(poses[0].weight, 0.4);
    assert_eq!(poses[0].node_id, node.id);
    assert_eq!(reopened.index().cover_asset_of(node.id).unwrap(), Some(other));

    // And the role that must never be sent is still that role after the round
    // trip — a rebuild that lost the distinction would be the leak.
    let moods = reopened.index().asset_links_in_role(node.id, AssetRole::Mood).unwrap();
    assert_eq!(moods.len(), 1);
    assert!(!moods[0].role.is_conditioning());
}

#[test]
fn an_external_edit_to_the_frontmatter_reaches_the_index() {
    // Obsidian, a git pull, or a collaborator on the share. The index is a
    // cache of the file, so a role added by hand has to become queryable
    // without anyone going through a command.
    let (_dir, mut project, node, asset, _other) = world();
    let rel = project.index().rel_path_of(node.id).unwrap().unwrap();
    let path = wobu_store::paths::from_rel_string(project.root(), &rel);

    let text = fs::read_to_string(&path).unwrap();
    let (frontmatter, body) = text.split_once("\n---\n").unwrap();
    fs::write(
        &path,
        format!("{frontmatter}\nassets:\n  - asset: {asset}\n    role: costume\n---\n{body}"),
    )
    .unwrap();

    project.reconcile().unwrap();
    let links = project.index().asset_links_in_role(node.id, AssetRole::Costume).unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].asset_id, asset);
}

/* ── unlinking, and what it must not touch ────────────────────────────────── */

#[test]
fn unlinking_the_last_reference_leaves_the_blob_alone() {
    // Assets are content-addressed and shared: the file behind this link may be
    // the cover of another node, and is byte-identical to whatever a
    // collaborator imported. Removing the last link is not evidence anybody
    // wants the picture gone, and deleting it is not undoable.
    let (_dir, mut project, node, asset, _other) = world();
    project.link_asset(node.id, asset, AssetRole::Pose, None).unwrap();

    let rel_path = project.get_asset(asset).unwrap().unwrap().rel_path;
    let blob = project.root().join(&rel_path);
    let bytes = fs::read(&blob).unwrap();

    let node = saved(project.unlink_asset(node.id, asset, AssetRole::Pose).unwrap());
    assert!(node.asset_links.is_empty());
    assert!(blob.is_file(), "unlinking deleted the file");
    assert_eq!(fs::read(&blob).unwrap(), bytes);
    assert!(project.get_asset(asset).unwrap().is_some(), "and it is still in the library");
}

#[test]
fn deleting_a_node_takes_its_links_and_not_the_pictures() {
    let (_dir, mut project, node, asset, _other) = world();
    project.link_asset(node.id, asset, AssetRole::Pose, None).unwrap();
    let blob = project.root().join(project.get_asset(asset).unwrap().unwrap().rel_path);

    project.delete_node(node.id).unwrap();
    assert!(project.index().asset_backlinks(asset).unwrap().is_empty());
    assert!(blob.is_file(), "deleting a node deleted a shared picture");
}

#[test]
fn one_picture_can_be_referenced_by_several_nodes() {
    // The reason unlinking must not delete: dedup means two nodes referencing
    // "the same picture" are referencing one file.
    let (_dir, mut project, kael, asset, _other) = world();
    let oru = project.create_node(NodeKind::Character, "Sister Oru", None).unwrap();
    project.link_asset(kael.id, asset, AssetRole::Costume, None).unwrap();
    project.link_asset(oru.id, asset, AssetRole::Costume, None).unwrap();

    let users = project.index().asset_backlinks(asset).unwrap();
    assert_eq!(users.len(), 2);

    project.unlink_asset(kael.id, asset, AssetRole::Costume).unwrap();
    assert_eq!(project.index().asset_backlinks(asset).unwrap().len(), 1);
    assert_eq!(project.get_node(oru.id).unwrap().asset_links.len(), 1, "Oru kept hers");
}

/* ── concurrency ──────────────────────────────────────────────────────────── */

#[test]
fn attaching_a_reference_loses_a_save_race_like_any_other_edit() {
    // Links are an edit to the node's Markdown, so they go through the guarded
    // write. A private path here would be a way to clobber a collaborator that
    // the editor itself does not have.
    let (_dir, mut project, node, asset, _other) = world();
    let rel = project.index().rel_path_of(node.id).unwrap().unwrap();
    let path = wobu_store::paths::from_rel_string(project.root(), &rel);

    // Somebody else writes the file after we last read it.
    let text = fs::read_to_string(&path).unwrap();
    let (frontmatter, _) = text.split_once("\n---\n").unwrap();
    fs::write(&path, format!("{frontmatter}\n---\n\n## Notes\n\nnadia got here first\n")).unwrap();

    match project.link_asset(node.id, asset, AssetRole::Pose, None).unwrap() {
        SaveOutcome::Conflict { conflict_path } => {
            assert!(project.root().join(&conflict_path).is_file(), "ours should be parked");
            assert!(
                fs::read_to_string(&path).unwrap().contains("nadia got here first"),
                "theirs must still be the file on disk",
            );
        }
        SaveOutcome::Saved(_) => panic!("a concurrent write should not have been clobbered"),
    }
}

#[test]
fn a_link_is_refused_while_the_share_is_away() {
    // The same trap as any other write: a save into an unmounted share's
    // leftover mountpoint succeeds, lands on local disk, and is shadowed the
    // moment the share comes back — taking the link with it.
    let (_dir, mut project, node, asset, _other) = world();
    fs::remove_dir_all(project.root()).unwrap();
    fs::create_dir_all(project.root()).unwrap();

    for outcome in [
        project.link_asset(node.id, asset, AssetRole::Pose, None),
        project.unlink_asset(node.id, asset, AssetRole::Pose),
        project.update_asset_link(node.id, asset, AssetRole::Pose, Some(0.5), None),
        project.set_cover_asset(node.id, Some(asset)),
    ] {
        assert!(matches!(outcome.unwrap_err(), Error::Disconnected));
    }
}

/* ── the record shape ─────────────────────────────────────────────────────── */

#[test]
fn a_link_written_by_hand_survives_a_save_by_the_app() {
    // Someone attaching a reference in Obsidian and then editing the node in
    // Wobu must not lose the reference: `save_node` writes the whole node, so a
    // field it did not read would be silently dropped.
    let (_dir, mut project, node, asset, other) = world();
    let mut edited = project.get_node(node.id).unwrap();
    edited.asset_links.push(AssetRef::new(asset, AssetRole::Material));
    edited.cover_asset_id = Some(other);
    project.save_node(edited).unwrap();

    let mut again = project.get_node(node.id).unwrap();
    again.notes_raw = "typed in the editor".into();
    let after = saved(project.save_node(again).unwrap());

    assert_eq!(after.asset_links.len(), 1, "an unrelated save dropped the link");
    assert_eq!(after.asset_links[0].role, AssetRole::Material);
    assert_eq!(after.cover_asset_id, Some(other));
}
