//! One project folder, exercised through its public surface.

use super::reconcile::markdown_files_at;
use super::*;
use crate::assets::png;
use crate::error::Error;
use crate::paths;
use std::path::Path;
use wobu_core::new_id;
use wobu_core::{AssetKind, AssetRole, Generation, Id, NodeKind};

fn new_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}

#[test]
fn create_lays_out_a_self_contained_folder() {
    let (dir, project) = new_project();
    let root = dir.path().join("ashfall.wobu");
    assert_eq!(project.root(), root);
    for expected in [
        "project.json",
        "nodes",
        "assets/originals",
        "assets/thumbs",
        "generations",
        ".wobu/tmp",
        ".wobu/sessions",
    ] {
        assert!(root.join(expected).exists(), "missing {expected}");
    }
}

#[test]
fn create_seeds_the_two_singletons() {
    let (_dir, project) = new_project();
    let nodes = project.list_nodes().unwrap();
    let kinds: Vec<_> = nodes.iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&NodeKind::StyleGuide));
    assert!(kinds.contains(&NodeKind::WorldBible));
    assert_eq!(nodes.len(), 2);
}

#[test]
fn singletons_cannot_be_duplicated_or_deleted() {
    let (_dir, mut project) = new_project();
    assert!(project.create_node(NodeKind::StyleGuide, "Another Style", None).is_err());

    let style =
        project.list_nodes().unwrap().into_iter().find(|n| n.kind == NodeKind::StyleGuide).unwrap();
    assert!(project.delete_node(style.id).is_err());
}

#[test]
fn creating_a_project_twice_in_the_same_place_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    Project::create(dir.path(), "Ashfall").unwrap();
    assert!(matches!(Project::create(dir.path(), "Ashfall"), Err(Error::AlreadyExists(_))));
}

#[test]
fn a_node_round_trips_through_disk() {
    let (_dir, mut project) = new_project();
    let created = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();

    let mut edited = project.get_node(created.id).unwrap();
    assert_eq!(edited, created);

    edited.notes_raw = "scarred, ex-guild".into();
    let SaveOutcome::Saved(_) = project.save_node(edited).unwrap() else {
        panic!("expected a clean save")
    };

    let reloaded = project.get_node(created.id).unwrap();
    assert_eq!(reloaded.notes_raw, "scarred, ex-guild");
}

#[test]
fn generations_are_append_only_and_indexed_per_node() {
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let make = |id: &str, prompt: &str| Generation {
        id: Id::from_string(id).unwrap(),
        node_id: node.id,
        created_at: "2026-07-31T14:22:11Z".parse().unwrap(),
        preset: "character_sheet".into(),
        view_type: None,
        user_prompt: "at dusk".into(),
        compiled_prompt: prompt.into(),
        negative_prompt: "text, watermark".into(),
        backend: "gemini".into(),
        model: "gemini-2.5-flash-image".into(),
        seed: 42,
        params: Default::default(),
        output_asset_ids: vec![],
        influence_snapshot: wobu_core::InfluenceSnapshot { layers: vec![] },
    };
    let first = make("01ARZ3NDEKTSV4RRFFQ69G5FAV", "first compiled prompt");
    let second = make("01ARZ3NDEKTSV4RRFFQ69G5FAW", "second compiled prompt");

    project.record_generation(first.clone()).unwrap();
    project.record_generation(second.clone()).unwrap();
    let history = project.list_generations(node.id).unwrap();
    assert_eq!(history.len(), 2, "one node may have concurrent generation attempts");
    assert!(history.contains(&first));
    assert!(history.contains(&second));

    let changed = make("01ARZ3NDEKTSV4RRFFQ69G5FAV", "must not replace the first");
    assert!(matches!(project.record_generation(changed), Err(Error::AlreadyExists(_))));
    assert_eq!(project.get_generation(first.id).unwrap(), Some(first));

    project.index.clear().unwrap();
    assert!(project.list_generations(node.id).unwrap().is_empty());
    project.rescan().unwrap();
    assert_eq!(project.list_generations(node.id).unwrap().len(), 2);
}

/// One recorded concept, with whatever it produced.
fn concept(node_id: Id, outputs: Vec<Id>) -> Generation {
    Generation {
        id: new_id(),
        node_id,
        created_at: "2026-07-31T14:22:11Z".parse().unwrap(),
        preset: "portrait".into(),
        view_type: None,
        user_prompt: String::new(),
        compiled_prompt: "Kael".into(),
        negative_prompt: String::new(),
        backend: "comfyui".into(),
        model: "local".into(),
        seed: 42,
        params: Default::default(),
        output_asset_ids: outputs,
        influence_snapshot: wobu_core::InfluenceSnapshot { layers: vec![] },
    }
}

#[test]
fn deleting_a_concept_takes_the_picture_it_produced() {
    // Otherwise the concept is deleted from Concepts and still sitting in
    // the Asset Library and on the board, which is not what the button says.
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let output = project.import_asset(&png(64, 64), AssetKind::Generated).unwrap().asset;
    let generation = concept(node.id, vec![output.id]);
    project.record_generation(generation.clone()).unwrap();

    project.delete_generation(generation.id).unwrap();

    assert!(project.get_generation(generation.id).unwrap().is_none());
    assert!(project.get_asset(output.id).unwrap().is_none());
    assert!(!paths::from_rel_string(project.root(), &output.rel_path).exists());
}

#[test]
fn deleting_a_concept_leaves_an_output_the_user_kept() {
    // Pinning a result as a reference, or making it a cover, is the user
    // saying they want the picture for its own sake. Deleting the receipt
    // it came from is not them changing their mind about that.
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let pinned = project.import_asset(&png(64, 64), AssetKind::Generated).unwrap().asset;
    let cover = project.import_asset(&png(65, 65), AssetKind::Generated).unwrap().asset;
    let loose = project.import_asset(&png(66, 66), AssetKind::Generated).unwrap().asset;
    project.link_asset(node.id, pinned.id, AssetRole::FullRef, None).unwrap();
    project.set_cover_asset(node.id, Some(cover.id)).unwrap();
    let generation = concept(node.id, vec![pinned.id, cover.id, loose.id]);
    project.record_generation(generation.clone()).unwrap();

    project.delete_generation(generation.id).unwrap();

    assert!(project.get_asset(pinned.id).unwrap().is_some());
    assert!(project.get_asset(cover.id).unwrap().is_some());
    assert!(project.get_asset(loose.id).unwrap().is_none());
}

#[test]
fn deleting_a_concept_leaves_an_output_another_receipt_still_shows() {
    // Assets are content-addressed, so two runs that produced the same
    // bytes are one file. Deleting one of those concepts must not blank the
    // tile of the other.
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let shared = project.import_asset(&png(64, 64), AssetKind::Generated).unwrap().asset;
    let first = concept(node.id, vec![shared.id]);
    let second = concept(node.id, vec![shared.id]);
    project.record_generation(first.clone()).unwrap();
    project.record_generation(second.clone()).unwrap();

    project.delete_generation(first.id).unwrap();
    assert!(project.get_asset(shared.id).unwrap().is_some());

    // And once the last receipt showing it is gone, so is the picture.
    project.delete_generation(second.id).unwrap();
    assert!(project.get_asset(shared.id).unwrap().is_none());
}

#[test]
fn replay_receipt_may_outlive_its_original_node() {
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let original = concept(node.id, vec![]);
    project.record_generation(original.clone()).unwrap();
    project.delete_node(node.id).unwrap();
    let mut replay = original.clone();
    replay.id = new_id();
    replay.params.insert("replayOf".into(), serde_json::json!(original.id));
    project.record_replay_generation(replay.clone()).unwrap();
    assert_eq!(project.get_generation(replay.id).unwrap(), Some(replay));
}

#[test]
fn nodes_land_at_the_documented_path() {
    let (dir, mut project) = new_project();
    project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    assert!(dir.path().join("ashfall.wobu/nodes/character/kael-vantris.md").is_file());
}

#[test]
fn duplicate_names_get_distinct_filenames() {
    let (_dir, mut project) = new_project();
    let a = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let b = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    assert_ne!(a.slug, b.slug);
    assert_eq!(a.name, b.name);
}

#[test]
fn renaming_a_node_does_not_move_its_file() {
    // Moving the file out from under a collaborator is worse than a stale slug.
    let (_dir, mut project) = new_project();
    let mut node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    node.name = "Kael the Ashbound".into();
    project.save_node(node.clone()).unwrap();

    let reloaded = project.get_node(node.id).unwrap();
    assert_eq!(reloaded.slug, "kael-vantris");
    assert_eq!(reloaded.name, "Kael the Ashbound");
}

#[test]
fn deleting_a_parent_promotes_its_children() {
    let (_dir, mut project) = new_project();
    let region = project.create_node(NodeKind::Setting, "Ember Coast", None).unwrap();
    let city = project.create_node(NodeKind::Setting, "Cinder Bay", Some(region.id)).unwrap();

    project.delete_node(region.id).unwrap();

    let survivor = project.get_node(city.id).unwrap();
    assert_eq!(survivor.parent_id, None, "the city must not vanish with the region");
    assert!(project.get_node(region.id).is_err());
}

#[test]
fn explicit_node_links_add_update_remove_and_answer_backlinks() {
    let (_dir, mut project) = new_project();
    let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
    let kael = project.create_node(NodeKind::Character, "Kael", None).unwrap();

    let SaveOutcome::Saved(saved) = project
        .add_node_link(kael.id, guild.id, wobu_core::LinkRole::MemberOf, Some(2.0), Some(false))
        .unwrap()
    else {
        panic!("expected a clean add")
    };
    assert_eq!(saved.links.len(), 1);
    assert_eq!(saved.links[0].weight, 1.0, "command weights are clamped");
    assert!(!saved.links[0].enabled);
    let incoming = project.node_backlinks(guild.id).unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].from_id, kael.id);

    let SaveOutcome::Saved(saved) = project
        .update_node_link(kael.id, guild.id, wobu_core::LinkRole::MemberOf, Some(0.4), Some(true))
        .unwrap()
    else {
        panic!("expected a clean update")
    };
    assert_eq!(saved.links[0].weight, 0.4);
    assert!(saved.links[0].enabled);

    let SaveOutcome::Saved(saved) =
        project.remove_node_link(kael.id, guild.id, wobu_core::LinkRole::MemberOf).unwrap()
    else {
        panic!("expected a clean removal")
    };
    assert!(saved.links.is_empty());
    assert!(project.node_backlinks(guild.id).unwrap().is_empty());
}

#[test]
fn node_link_add_obeys_the_source_kinds_registry_roles() {
    let (_dir, mut project) = new_project();
    let style = project
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let kael = project.create_node(NodeKind::Character, "Kael", None).unwrap();

    let result =
        project.add_node_link(kael.id, style.id, wobu_core::LinkRole::StyledBy, None, None);
    assert!(matches!(result, Err(Error::InvalidNodeLinkRole { .. })));
    assert!(project.get_node(kael.id).unwrap().links.is_empty());
}

#[test]
fn deleting_a_node_strips_the_links_that_pointed_at_it() {
    let (_dir, mut project) = new_project();
    let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
    let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael.links.push(wobu_core::Link::new(guild.id, wobu_core::LinkRole::MemberOf));
    let SaveOutcome::Saved(kael) = project.save_node(kael).unwrap() else {
        panic!("expected a clean save")
    };

    project.delete_node(guild.id).unwrap();

    // Not just in the index — in the Markdown, which is the source of truth.
    let reread = project.get_node(kael.id).unwrap();
    assert!(reread.links.is_empty(), "dangling link survived: {:?}", reread.links);
    assert!(project.index().backlinks(guild.id).unwrap().is_empty());
}

#[test]
fn deleting_two_linked_nodes_works_in_either_order() {
    let (_dir, mut project) = new_project();
    let guild = project.create_node(NodeKind::Culture, "Ember Guild", None).unwrap();
    let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael.links.push(wobu_core::Link::new(guild.id, wobu_core::LinkRole::MemberOf));
    let SaveOutcome::Saved(kael) = project.save_node(kael).unwrap() else {
        panic!("expected a clean save")
    };

    project.delete_node(kael.id).unwrap();
    project.delete_node(guild.id).unwrap();
    assert!(project.list_nodes().unwrap().iter().all(|n| n.id != guild.id));
}

#[test]
fn a_move_that_would_make_a_cycle_is_refused() {
    let (_dir, mut project) = new_project();
    let region = project.create_node(NodeKind::Setting, "Ember Coast", None).unwrap();
    let city = project.create_node(NodeKind::Setting, "Cinder Bay", Some(region.id)).unwrap();

    assert!(project.move_node(region.id, Some(city.id)).is_err());
    assert_eq!(project.get_node(region.id).unwrap().parent_id, None);
}

#[test]
fn reopening_reads_the_world_back_off_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = {
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        project.root().to_path_buf()
    };

    let reopened = Project::open(&root).unwrap();
    let names: Vec<_> = reopened.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
    assert!(names.contains(&"Vashk".to_string()));
}

#[test]
fn deleting_the_index_loses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (root, id) = {
        let mut project = Project::create(dir.path(), "Ashfall").unwrap();
        project.create_node(NodeKind::Species, "Vashk", None).unwrap();
        (project.root().to_path_buf(), project.id())
    };

    std::fs::remove_file(paths::index_path(&id)).ok();
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.list_nodes().unwrap().len(), 3, "2 singletons + Vashk");
}

#[test]
fn reconcile_picks_up_an_external_edit() {
    // The Obsidian / git-pull / collaborator case.
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");

    let text = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
    // Push mtime forward; a same-second write can otherwise look unchanged.
    std::fs::write(&path, text).unwrap();
    filetime_bump(&path);

    assert!(project.reconcile().unwrap());
    let names: Vec<_> = project.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
    assert!(names.contains(&"Vashk-Prime".to_string()), "{names:?}");
    assert_eq!(project.get_node(node.id).unwrap().name, "Vashk-Prime");
}

#[test]
fn local_reconcile_only_reads_the_reported_node_path() {
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let sunborn = project.create_node(NodeKind::Species, "Sunborn", None).unwrap();
    let vashk_path = project.root().join("nodes/species/vashk.md");
    let sunborn_path = project.root().join("nodes/species/sunborn.md");

    for (path, before, after) in [
        (&vashk_path, "name: Vashk", "name: Vashk-Prime"),
        (&sunborn_path, "name: Sunborn", "name: Sunborn-Prime"),
    ] {
        let text = std::fs::read_to_string(path).unwrap().replace(before, after);
        std::fs::write(path, text).unwrap();
        filetime_bump(path);
    }

    assert!(project.reconcile_paths(std::slice::from_ref(&vashk_path)).unwrap());
    let indexed = project.list_nodes().unwrap();
    assert_eq!(indexed.iter().find(|node| node.id == vashk.id).unwrap().name, "Vashk-Prime");
    assert_eq!(
        indexed.iter().find(|node| node.id == sunborn.id).unwrap().name,
        "Sunborn",
        "an unrelated external edit must wait for its own event"
    );

    assert!(project.reconcile().unwrap());
    let indexed = project.list_nodes().unwrap();
    assert_eq!(indexed.iter().find(|node| node.id == sunborn.id).unwrap().name, "Sunborn-Prime");
}

#[test]
fn a_full_observation_is_rejected_if_a_file_moves_before_apply() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");

    let first = std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: First");
    std::fs::write(&path, first).unwrap();
    filetime_bump(&path);
    let observation = project.reconcile_plan().unwrap().observe().unwrap();

    let second = std::fs::read_to_string(&path).unwrap().replace("name: First", "name: Second");
    std::fs::write(&path, second).unwrap();
    filetime_bump(&path);

    assert!(!observation.revalidate().unwrap());
    assert!(project.reconcile().unwrap());
    assert!(project.list_nodes().unwrap().iter().any(|node| node.name == "Second"));
}

#[test]
fn revalidation_notices_a_file_that_was_unchanged_during_observation() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    let observation = project.reconcile_plan().unwrap().observe().unwrap();

    let edited =
        std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Vashk-Prime");
    std::fs::write(&path, edited).unwrap();
    filetime_bump(&path);

    assert!(!observation.revalidate().unwrap());
    assert!(project.reconcile().unwrap());
}

#[test]
fn markdown_walk_does_not_descend_below_the_node_depth_limit() {
    let (_dir, project) = new_project();
    let deep = project.root().join("nodes/species/deep/deeper");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("hidden.md"), "---\nname: Hidden\n---\n").unwrap();

    assert!(markdown_files_at(project.root()).iter().all(|(rel, _)| !rel.ends_with("hidden.md")));
}

#[test]
fn reconcile_survives_two_files_swapping_names() {
    // Renaming files around in Obsidian is normal, and a swap is the case
    // where a node arrives at a path the index still believes belongs to
    // someone else. Getting this wrong makes the project fail to open.
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let sunborn = project.create_node(NodeKind::Species, "Sunborn", None).unwrap();

    let dir = project.root().join("nodes/species");
    let (a, b, tmp) = (dir.join("vashk.md"), dir.join("sunborn.md"), dir.join("swap.tmp"));
    std::fs::rename(&a, &tmp).unwrap();
    std::fs::rename(&b, &a).unwrap();
    std::fs::rename(&tmp, &b).unwrap();
    filetime_bump(&a);
    filetime_bump(&b);

    project.reconcile().unwrap();

    // The slug follows the filename, so they have traded places.
    assert_eq!(project.get_node(vashk.id).unwrap().slug, "sunborn");
    assert_eq!(project.get_node(sunborn.id).unwrap().slug, "vashk");
}

#[test]
fn reconcile_notices_an_externally_deleted_file() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    std::fs::remove_file(project.root().join("nodes/species/vashk.md")).unwrap();

    assert!(project.reconcile().unwrap());
    assert_eq!(project.list_nodes().unwrap().len(), 2, "only the singletons remain");
}

/// A truncated YAML frontmatter block — the shape Dropbox and OneDrive
/// actually produce when they copy a half-written file.
fn truncate_frontmatter(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let cut = text.len() / 3;
    std::fs::write(path, &text[..cut]).unwrap();
    filetime_bump(path);
}

#[test]
fn a_mangled_file_is_recorded_rather_than_dropped() {
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    truncate_frontmatter(&path);

    assert!(project.reconcile().unwrap());

    let corrupt = project.corrupt_files().unwrap();
    assert_eq!(corrupt.len(), 1, "{corrupt:?}");
    assert_eq!(corrupt[0].rel_path, "nodes/species/vashk.md");
    assert_eq!(corrupt[0].node_id, Some(vashk.id), "the broken file is tied to its entity");
    assert!(!corrupt[0].error.is_empty(), "the parse error is what tells the user what to fix");

    // The parser names the file by absolute path. That is right for a log
    // and wrong for a string the UI shows and a user pastes into a bug
    // report — the leading half of it is their home directory.
    let root = project.root().to_string_lossy().into_owned();
    assert!(!corrupt[0].error.contains(&root), "leaked an absolute path: {}", corrupt[0].error);
    assert!(
        corrupt[0].error.contains("nodes/species/vashk.md"),
        "the file should still be named, relatively: {}",
        corrupt[0].error,
    );
}

#[test]
fn a_mangled_file_keeps_its_node_row_and_its_bytes() {
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    truncate_frontmatter(&path);
    let on_disk = std::fs::read(&path).unwrap();

    project.reconcile().unwrap();

    // The row survives — this is the whole point. A live node beside a
    // broken file is how the user finds their data again; dropping it
    // makes the entity silently cease to exist.
    let listed = project.list_nodes().unwrap();
    assert!(listed.iter().any(|n| n.id == vashk.id), "the node vanished from the navigator");
    assert_eq!(std::fs::read(&path).unwrap(), on_disk, "the file was modified");
}

#[test]
fn a_mangled_file_is_never_written_over() {
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    truncate_frontmatter(&path);
    let on_disk = std::fs::read(&path).unwrap();
    project.reconcile().unwrap();

    // Saving the last-known-good node over the mangled file would destroy
    // whatever the sync client left behind — including, possibly, the only
    // copy of an edit made on another machine.
    let outcome = project.save_node(vashk).unwrap();
    assert!(matches!(outcome, SaveOutcome::Conflict { .. }), "{outcome:?}");
    assert_eq!(std::fs::read(&path).unwrap(), on_disk, "the mangled file was overwritten");
}

#[test]
fn a_repaired_file_stops_being_corrupt() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    let good = std::fs::read_to_string(&path).unwrap();

    truncate_frontmatter(&path);
    project.reconcile().unwrap();
    assert_eq!(project.corrupt_files().unwrap().len(), 1);

    // The user restored it from a backup, or the sync client finished.
    std::fs::write(&path, &good).unwrap();
    filetime_bump(&path);
    project.reconcile().unwrap();

    assert!(project.corrupt_files().unwrap().is_empty(), "the broken state stuck around");
    assert_eq!(project.list_nodes().unwrap().len(), 3);
}

#[test]
fn deleting_a_mangled_file_clears_it() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    truncate_frontmatter(&path);
    project.reconcile().unwrap();
    assert_eq!(project.corrupt_files().unwrap().len(), 1);

    // Giving up on it is a legitimate resolution, and the banner has to go.
    std::fs::remove_file(&path).unwrap();
    project.reconcile().unwrap();
    assert!(project.corrupt_files().unwrap().is_empty());
}

/// The counterpart to the test above, and the distinction the whole
/// unmount story rests on: a deleted *file* is a real deletion, a missing
/// *folder* is not evidence of anything.
#[test]
fn reconcile_refuses_to_read_a_vanished_share_as_mass_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let before = project.list_nodes().unwrap().len();

    // What an unmount leaves behind: the mountpoint is still a directory,
    // so `root.is_dir()` is true and walking it yields nothing.
    std::fs::remove_dir_all(project.root()).unwrap();
    std::fs::create_dir_all(project.root()).unwrap();
    assert!(project.root().is_dir(), "the mountpoint should still look like a directory");

    assert!(matches!(project.reconcile(), Err(Error::Disconnected)));
    assert_eq!(
        project.list_nodes().unwrap().len(),
        before,
        "the index is the only readable copy of the world while the share is away",
    );
}

/// The index holds summaries, not bodies, so a node that was never opened
/// before the share went cannot be read. What it must not do is claim the
/// node does not exist.
#[test]
fn an_unreadable_node_blames_the_share_rather_than_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();

    std::fs::remove_dir_all(project.root()).unwrap();
    std::fs::create_dir_all(project.root()).unwrap();

    assert!(matches!(project.get_node(vashk.id), Err(Error::Disconnected)));
    // Still listed, because that comes from the index.
    assert!(project.list_nodes().unwrap().iter().any(|n| n.id == vashk.id));
}

#[test]
fn a_genuinely_missing_node_still_says_so() {
    let (_dir, mut project) = new_project();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    std::fs::remove_file(project.root().join("nodes/species/vashk.md")).unwrap();

    // The folder is fine, so the file being gone is real information.
    assert!(matches!(project.get_node(vashk.id), Err(Error::NoSuchNode(_))));
}

#[test]
fn writes_are_refused_while_the_share_is_away() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap();

    std::fs::remove_dir_all(project.root()).unwrap();
    std::fs::create_dir_all(project.root()).unwrap();

    // Left to itself this write would *succeed*, landing on the local disk
    // under the empty mountpoint — invisible to everyone else and shadowed
    // the moment the share returns.
    assert!(matches!(project.save_node(vashk), Err(Error::Disconnected)));
    assert!(matches!(
        project.create_node(NodeKind::Species, "Sunborn", None),
        Err(Error::Disconnected)
    ));
}

#[test]
fn a_share_that_comes_back_reconciles_normally() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();

    let stashed = dir.path().join("stash");
    std::fs::rename(project.root(), &stashed).unwrap();
    assert!(matches!(project.reconcile(), Err(Error::Disconnected)));

    std::fs::rename(&stashed, project.root()).unwrap();
    assert!(project.is_present());
    project.reconcile().unwrap();
    assert_eq!(project.list_nodes().unwrap().len(), 3, "singletons plus Vashk");
}

#[test]
fn conflict_siblings_are_never_indexed_as_nodes() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let original = project.root().join("nodes/species/vashk.md");
    let sibling = project.root().join("nodes/species/vashk.conflict-nadia-20260731T142211Z.md");
    std::fs::copy(&original, &sibling).unwrap();

    project.rescan().unwrap();
    let vashk: Vec<_> =
        project.list_nodes().unwrap().into_iter().filter(|n| n.name == "Vashk").collect();
    assert_eq!(vashk.len(), 1, "the conflict copy must not appear in the navigator");
}

#[test]
fn a_corrupt_file_is_skipped_rather_than_overwritten() {
    let (_dir, mut project) = new_project();
    project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");
    std::fs::write(&path, "this is not a node file").unwrap();

    project.rescan().unwrap();
    assert_eq!(project.list_nodes().unwrap().len(), 2, "corrupt file drops out of the index");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "this is not a node file",
        "but is left exactly as it was on disk"
    );
}

#[test]
fn a_concurrent_edit_produces_a_conflict_not_a_clobber() {
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    let path = project.root().join("nodes/species/vashk.md");

    // Nadia saves while we hold an older copy.
    let theirs =
        std::fs::read_to_string(&path).unwrap().replace("name: Vashk", "name: Nadia's Vashk");
    std::fs::write(&path, theirs).unwrap();
    filetime_bump(&path);

    let mut mine = node.clone();
    mine.notes_raw = "my edit".into();
    let outcome = project.save_node(mine).unwrap();

    let SaveOutcome::Conflict { conflict_path } = outcome else {
        panic!("expected a conflict, got {outcome:?}")
    };
    assert!(conflict_path.contains(".conflict-"), "{conflict_path}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("Nadia's Vashk"));
    assert!(project.root().join(&conflict_path).is_file());
}

#[test]
fn a_read_only_project_refuses_writes_rather_than_failing_late() {
    let (_dir, mut project) = new_project();
    project.read_only = true;
    assert!(matches!(project.create_node(NodeKind::Species, "Vashk", None), Err(Error::ReadOnly)));
}

#[test]
fn a_read_only_project_refuses_an_import_rather_than_failing_late() {
    // An import is a write like any other, and the chip in the title bar
    // has already told the user this folder cannot take one.
    let (_dir, mut project) = new_project();
    project.read_only = true;
    assert!(matches!(
        project.import_asset(&[0x89, b'P', b'N', b'G'], AssetKind::Reference),
        Err(Error::ReadOnly)
    ));
}

#[test]
fn a_read_only_project_refuses_an_asset_link_rather_than_failing_late() {
    // Attaching a reference is an edit to the node file, so the chip in the
    // title bar has already told the user this cannot work.
    let (_dir, mut project) = new_project();
    let node = project.create_node(NodeKind::Character, "Kael", None).unwrap();
    let asset = wobu_core::new_id();
    project.read_only = true;

    assert!(matches!(
        project.link_asset(node.id, asset, AssetRole::Pose, None),
        Err(Error::ReadOnly)
    ));
    assert!(matches!(project.set_cover_asset(node.id, Some(asset)), Err(Error::ReadOnly)));
    // Read-only is reported ahead of the asset not existing: the folder is
    // the reason nothing can happen here, and it is the one the user can do
    // something about.
    assert!(matches!(project.unlink_asset(node.id, asset, AssetRole::Pose), Err(Error::ReadOnly)));
}

#[test]
fn an_import_is_refused_while_the_share_is_away() {
    // The same trap as a node save: writing into an unmounted share's
    // leftover mountpoint succeeds, lands on local disk, and is shadowed
    // the moment the share comes back — taking the reference with it.
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    std::fs::remove_dir_all(project.root()).unwrap();
    std::fs::create_dir_all(project.root()).unwrap();

    assert!(matches!(
        project.import_asset(&[0x89, b'P', b'N', b'G'], AssetKind::Reference),
        Err(Error::Disconnected)
    ));
}

#[test]
fn opening_a_plain_folder_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(Project::open(dir.path()), Err(Error::NotAProject(_))));
}

#[test]
fn a_newer_schema_is_refused_rather_than_misread() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    let meta_path = project.root().join(PROJECT_FILE);
    let raw = std::fs::read_to_string(&meta_path).unwrap();
    std::fs::write(&meta_path, raw.replace("\"schemaVersion\": 1", "\"schemaVersion\": 99"))
        .unwrap();

    assert!(matches!(Project::open(project.root()), Err(Error::SchemaTooNew { found: 99, .. })));
}

#[test]
fn nothing_absolute_is_written_into_the_folder() {
    // The same share is /Volumes/art/… on one machine and Z:\art\… on another.
    let (dir, mut project) = new_project();
    project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let root_string = dir.path().to_string_lossy().into_owned();

    for (_, path) in project.node_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&root_string), "{} leaked an absolute path", path.display());
    }
    let meta = std::fs::read_to_string(project.root().join(PROJECT_FILE)).unwrap();
    assert!(!meta.contains(&root_string));
}

#[test]
fn spend_ceiling_is_shared_and_preserves_unknown_metadata() {
    let (_dir, mut project) = new_project();
    assert_eq!(project.meta().spend_ceiling_usd_micros, Some(DEFAULT_SPEND_CEILING_USD_MICROS));
    let path = project.root().join(PROJECT_FILE);
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    meta.as_object_mut()
        .unwrap()
        .insert("futureMetadata".into(), serde_json::json!({ "kept": true }));
    std::fs::write(&path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();

    project.set_spend_ceiling(Some(2_500_000)).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["spendCeilingUsdMicros"], 2_500_000);
    assert_eq!(saved["futureMetadata"]["kept"], true);
    assert_eq!(project.meta().spend_ceiling_usd_micros, Some(2_500_000));

    project.set_spend_ceiling(None).unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
            .unwrap()["spendCeilingUsdMicros"]
            .is_null()
    );
}

#[test]
fn project_without_a_ceiling_gets_the_default_guardrail() {
    let (_dir, project) = new_project();
    let path = project.root().join(PROJECT_FILE);
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    meta.as_object_mut().unwrap().remove("spendCeilingUsdMicros");
    std::fs::write(&path, serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
    let root = project.root().to_path_buf();
    drop(project);
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.meta().spend_ceiling_usd_micros, Some(DEFAULT_SPEND_CEILING_USD_MICROS));
}

#[test]
fn transfer_reports_a_guarded_write_race_with_pending_ids() {
    let source_dir = tempfile::tempdir().unwrap();
    let mut source = Project::create(source_dir.path(), "Source").unwrap();
    let source_style = source
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let mut style = source.get_node(source_style.id).unwrap();
    style.notes_raw = "the incoming house style".to_string();
    assert!(matches!(source.save_node(style).unwrap(), SaveOutcome::Saved(_)));
    let source_root = source.root().to_path_buf();
    drop(source);

    let bundle = crate::transfer::stage(&source_root, source_style.id).unwrap();
    let (_destination_dir, mut destination) = new_project();
    let destination_style = destination
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();

    let outcome = destination
        .apply_transfer_with(bundle, |project| {
            let mut changed = project.get_node(destination_style.id).unwrap();
            changed.notes_raw = "a collaborator won the race".to_string();
            assert!(matches!(project.save_node(changed).unwrap(), SaveOutcome::Saved(_)));
        })
        .unwrap();

    assert!(!outcome.completed);
    assert!(outcome.applied_node_ids.is_empty());
    assert_eq!(outcome.pending_node_ids, vec![destination_style.id]);
    assert_eq!(outcome.conflict_paths.len(), 1);
    assert!(outcome.failure.as_deref().unwrap().contains("parked"));
    assert_eq!(
        destination.get_node(destination_style.id).unwrap().notes_raw,
        "a collaborator won the race"
    );
}

/// Nudge mtime forward so a write inside the same filesystem timestamp
/// granularity is still visible to the `(mtime, size)` pre-filter.
fn filetime_bump(path: &Path) {
    let meta = std::fs::metadata(path).unwrap();
    let later = meta.modified().unwrap() + std::time::Duration::from_secs(2);
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(later).unwrap();
}
