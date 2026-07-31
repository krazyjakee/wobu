//! The store, against a real project folder nobody's writer produced.
//!
//! Every other test in this crate builds its world through the store's own API,
//! which means the writer and the reader can agree on something wrong and no
//! test notices. `examples/Ashfall.wobu` is hand-authored Markdown — the same
//! thing an Obsidian user or a `git pull` would hand us — so this is the only
//! test that can catch the format drifting away from what a human would write.

use std::path::{Path, PathBuf};

use wobu_core::{Id, LinkRole, NodeKind};
use wobu_store::{Project, markdown};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/Ashfall.wobu")
}

fn id(s: &str) -> Id {
    Id::from_string(s).expect("fixture ids are valid ULIDs")
}

/// Opening a project probes writability and stages temp files, so the folder is
/// copied out of the repo first — a test must not dirty a committed fixture.
fn open_a_copy() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Ashfall.wobu");
    copy_dir(&fixture(), &root);
    let project = Project::open(&root).expect("the example project should open");
    (dir, project)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Every `.md` under `nodes/`, so a file that fails to parse cannot hide by
/// simply being absent from the index.
fn node_files(root: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root.join("nodes"))
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.path().to_path_buf())
        .collect()
}

#[test]
fn the_example_project_opens_and_indexes_every_node() {
    let (_dir, project) = open_a_copy();
    let nodes = project.list_nodes().unwrap();

    // `rescan` skips files it cannot parse rather than destroying them, so a
    // count check is what turns a silent skip into a failure.
    assert_eq!(
        nodes.len(),
        node_files(project.root()).len(),
        "every file on disk should be indexed; got {:?}",
        nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
    assert_eq!(nodes.len(), 13);
}

#[test]
fn every_file_survives_a_read_write_round_trip_byte_for_byte() {
    // The one that matters. If a hand-authored file re-renders differently,
    // then opening the app and saving one node rewrites files nobody touched,
    // and every collaborator on the share sees a diff they did not make.
    let root = fixture();
    for path in node_files(&root) {
        let original = std::fs::read_to_string(&path).unwrap();
        let node = markdown::from_markdown(&original, &path)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));
        let rendered = markdown::to_markdown(&node).unwrap();
        assert_eq!(rendered, original, "{} is not stable under a save", path.display());
    }
}

#[test]
fn the_singletons_are_present_and_unique() {
    let (_dir, project) = open_a_copy();
    for kind in [NodeKind::StyleGuide, NodeKind::WorldBible] {
        assert!(
            project.index().singleton_of(kind).unwrap().is_some(),
            "{kind} singleton missing — every influence stack is rooted in it"
        );
    }
}

#[test]
fn influence_edges_all_resolve_to_real_nodes() {
    // A link to a node that does not exist would resolve into an empty layer
    // card rather than nothing at all.
    let (_dir, project) = open_a_copy();
    let known: Vec<Id> = project.list_nodes().unwrap().into_iter().map(|n| n.id).collect();

    for summary in project.list_nodes().unwrap() {
        let node = project.get_node(summary.id).unwrap();
        for link in &node.links {
            assert!(
                known.contains(&link.to_id),
                "{} links to a node that does not exist: {}",
                node.name,
                link.to_id
            );
        }
        if let Some(parent) = node.parent_id {
            assert!(known.contains(&parent), "{} has a missing parent", node.name);
        }
    }
}

#[test]
fn kael_carries_the_stack_the_prototype_described() {
    let (_dir, project) = open_a_copy();
    let kael = project.get_node(id("01KF0E44M90400K6CSK6CSK6CS")).unwrap();

    assert_eq!(kael.name, "Kael Vantris");
    assert_eq!(kael.kind, NodeKind::Character);
    assert_eq!(kael.description_state, wobu_core::DescriptionState::Stale);

    let roles: Vec<LinkRole> = kael.links.iter().map(|l| l.role).collect();
    assert_eq!(roles, vec![LinkRole::SpeciesOf, LinkRole::MemberOf, LinkRole::LocatedIn]);

    let description = kael.description.as_ref().expect("kael has a description");
    assert!(description.text("silhouette").unwrap().contains("permanent stoop"));
    assert_eq!(description.never().len(), 3, "never feeds the negative prompt");
}

#[test]
fn same_kind_nesting_survives_the_folder() {
    // Cinder Bay nests inside the Ember Coast; the Drowned Market is an
    // environment, which does not nest, so its containment is a link instead.
    let (_dir, project) = open_a_copy();

    let bay = project.get_node(id("01KF0E44M70400EXVQEXVQEXVQ")).unwrap();
    assert_eq!(bay.parent_id, Some(id("01KF0E44M60400CSK6CSK6CSK6")));

    let market = project.get_node(id("01KF0E44M80400H248H248H248")).unwrap();
    assert_eq!(market.parent_id, None, "environments do not nest");
    assert!(
        market.links.iter().any(|l| l.role == LinkRole::LocatedIn && l.to_id == bay.id),
        "the market should still be located in the bay"
    );
}

#[test]
fn a_node_never_enhanced_has_no_description() {
    // `none` means never enhanced, so a description block would contradict it.
    let (_dir, project) = open_a_copy();
    for ulid in ["01KF0E44M2040048H248H248H2", "01KF0E44M50400ANANANANANAN"] {
        let node = project.get_node(id(ulid)).unwrap();
        assert_eq!(node.description_state, wobu_core::DescriptionState::None);
        assert!(node.description.is_none(), "{} should have no description", node.name);
    }
}

#[test]
fn search_reaches_into_notes_and_descriptions() {
    let (_dir, project) = open_a_copy();

    // "ashglass" appears in Kael's description, not in any node's name.
    let hits = project.index().search("ashglass").unwrap();
    assert!(!hits.is_empty(), "full-text search should reach description prose");

    let names: Vec<String> = hits
        .iter()
        .filter_map(|id| project.get_node(*id).ok())
        .map(|n| n.name)
        .collect();
    assert!(names.iter().any(|n| n == "Ashglass Lantern"), "{names:?}");
}

#[test]
fn deleting_the_index_loses_nothing_from_the_folder() {
    let (_dir, project) = open_a_copy();
    let before: Vec<String> =
        project.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
    let root = project.root().to_path_buf();
    drop(project);

    // The index is a cache keyed by project ULID; blowing it away must be safe.
    let reopened = Project::open(&root).unwrap();
    reopened.index().clear().unwrap();
    let rebuilt = Project::open(&root).unwrap();

    let after: Vec<String> = rebuilt.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
    assert_eq!(after, before);
}
