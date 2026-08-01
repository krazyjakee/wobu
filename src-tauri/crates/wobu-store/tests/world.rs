//! The world the influence engine resolves against, proved from outside the
//! crate.
//!
//! `wobu-influence` is pure: it borrows already-loaded `Node`s and does no IO,
//! so that `prompt_compile` stays sub-millisecond on every drag of a weight
//! slider. `Project::world_nodes` is where those nodes come from, and it makes
//! four promises that the panel above it is built on:
//!
//! - it answers with the world as the index last saw it, whole — descriptions,
//!   links and references included, not the summaries the navigator renders;
//! - it never reads the project folder, so it is exactly as fast on a share
//!   that has just been unplugged as on a local disk, which is the state
//!   `docs/07-file-shares.md` promises the app keeps working in;
//! - one node changing costs one row, not a rebuild of the world;
//! - it belongs to the open project and dies with it, so nothing can be served
//!   from a project that has been closed or from a different one.

use std::fs;

use wobu_core::asset::AssetRef;
use wobu_core::{
    AssetKind, AssetRole, Description, Id, Link, LinkRole, Node, NodeKind, SectionValue,
};
use wobu_store::{Project, SaveOutcome};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A PNG header and nothing more than the parser reads.
fn png(width: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&480u32.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

/// A project with the two singletons, a culture and a character held to it.
fn ashfall() -> (tempfile::TempDir, Project, Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let guild = project.create_node(NodeKind::Culture, "Cinder Guild", None).unwrap();

    let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael.links = vec![Link::new(guild.id, LinkRole::MemberOf)];
    kael.description = Some(Description::from_sections([(
        "silhouette".to_string(),
        SectionValue::Text("Tall, narrow, hooded".into()),
    )]));
    let id = kael.id;
    saved(project.save_node(kael).unwrap());
    (dir, project, id)
}

fn saved(outcome: SaveOutcome) -> Node {
    match outcome {
        SaveOutcome::Saved(node) => *node,
        SaveOutcome::Conflict { conflict_path } => panic!("unexpected conflict at {conflict_path}"),
    }
}

fn named<'a>(nodes: &'a [Node], name: &str) -> &'a Node {
    nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("no node named {name}"))
}

/* ── what it hands back ───────────────────────────────────────────────────── */

#[test]
fn the_world_arrives_whole_rather_than_as_the_summaries_the_navigator_renders() {
    // The engine reads descriptions, links and references. A `NodeSummary` has
    // none of the three, so a stack built from the navigator's data source would
    // resolve to a list of names with nothing in them and never say why.
    let (_dir, mut project, kael_id) = ashfall();
    let nodes = project.world_nodes().unwrap().to_vec();

    let kael = named(&nodes, "Kael Vantris");
    assert_eq!(kael.id, kael_id);
    assert_eq!(
        kael.description.as_ref().and_then(|d| d.text("silhouette")),
        Some("Tall, narrow, hooded")
    );
    assert_eq!(kael.links.len(), 1);
    assert_eq!(kael.links[0].role, LinkRole::MemberOf);
    assert_eq!(kael.links[0].to_id, named(&nodes, "Cinder Guild").id);

    // And the roots of every stack are in it — `World` finds them by kind.
    let kinds: Vec<NodeKind> = nodes.iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&NodeKind::StyleGuide));
    assert!(kinds.contains(&NodeKind::WorldBible));
    assert_eq!(nodes.len(), 4);
}

#[test]
fn the_nodes_come_back_in_id_order_whatever_order_they_were_written_in() {
    // `World` picks the Style Guide by lowest id, so that a project that somehow
    // acquired two renders the same way on every machine. That guarantee is only
    // worth anything if the caller cannot change the answer by saving nodes in a
    // different order, which is what this holds.
    let (_dir, mut project, kael_id) = ashfall();
    let ids: Vec<Id> = project.world_nodes().unwrap().iter().map(|n| n.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);

    // Touch the oldest node last; the order must not move.
    let mut kael = project.get_node(kael_id).unwrap();
    kael.summary = "Ex-guild enforcer".into();
    saved(project.save_node(kael).unwrap());
    assert_eq!(project.world_nodes().unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), sorted);
}

#[test]
fn a_reference_image_and_its_role_survive_into_the_world() {
    // The role is what decides whether an image reaches a backend at all, and
    // `mood` means it never does. A world that dropped the roles would put that
    // decision back in the hands of whatever assembled the request.
    let (_dir, mut project, kael_id) = ashfall();
    let asset = project.import_asset(&png(640), AssetKind::Reference).unwrap().asset.id;
    saved(project.link_asset(kael_id, asset, AssetRole::Mood, None).unwrap());

    let nodes = project.world_nodes().unwrap();
    let kael = named(nodes, "Kael Vantris");
    assert_eq!(kael.asset_links, [AssetRef::new(asset, AssetRole::Mood)]);
}

/* ── what it costs ────────────────────────────────────────────────────────── */

#[test]
fn nothing_here_reads_the_project_folder() {
    // The whole point. The engine is sub-millisecond by construction, and a
    // read of every node file over SMB in front of it would make the panel the
    // slowest thing in the app while looking, from inside the engine, like
    // nothing had changed. Proved the only way it can be: take the folder away
    // and ask again.
    //
    // This is also the offline promise. A share can vanish at any moment, the
    // index lives in local app data, and the Inspector has to keep working from
    // it — see `docs/07-file-shares.md`.
    let (_dir, mut project, kael_id) = ashfall();
    let before = project.world_nodes().unwrap().len();

    fs::remove_dir_all(project.root().join("nodes")).unwrap();
    assert!(project.get_node(kael_id).is_err(), "the file really is gone");

    let nodes = project.world_nodes().unwrap();
    assert_eq!(nodes.len(), before);
    assert_eq!(
        named(nodes, "Kael Vantris").description.as_ref().and_then(|d| d.text("silhouette")),
        Some("Tall, narrow, hooded"),
    );
}

#[test]
fn one_node_changing_costs_one_node_and_not_the_world() {
    // "Do not rebuild everything when one node changed." The index records which
    // rows moved and this re-reads only those, which is what keeps a save made
    // while the Inspector is open from re-materialising a world of thousands.
    //
    // Measured by identity rather than by a clock: every node that did not
    // change is the same allocation it was before, which is only true if it was
    // never re-read. A rebuild would move all four.
    let (_dir, mut project, kael_id) = ashfall();
    let addresses = |nodes: &[Node]| -> Vec<(Id, usize)> {
        nodes.iter().map(|n| (n.id, n.name.as_ptr() as usize)).collect()
    };

    let before = addresses(project.world_nodes().unwrap());
    let mut kael = project.get_node(kael_id).unwrap();
    kael.summary = "Ex-guild enforcer".into();
    saved(project.save_node(kael).unwrap());
    let after = addresses(project.world_nodes().unwrap());

    let moved: Vec<Id> = before
        .iter()
        .zip(&after)
        .filter(|((_, a), (_, b))| a != b)
        .map(|((id, _), _)| *id)
        .collect();
    assert_eq!(moved, [kael_id], "only the saved node should have been re-read");
    assert_eq!(
        named(project.world_nodes().unwrap(), "Kael Vantris").summary,
        "Ex-guild enforcer",
        "and it should have been re-read",
    );
}

#[test]
fn asking_twice_with_nothing_in_between_does_no_work_at_all() {
    // The Inspector's steady state: a slider drag compiles against a world
    // nothing has touched. If this cost a query the panel would pay for the
    // whole index on every frame of a drag.
    let (_dir, mut project, _) = ashfall();
    let first: Vec<usize> =
        project.world_nodes().unwrap().iter().map(|n| n.name.as_ptr() as usize).collect();
    let second: Vec<usize> =
        project.world_nodes().unwrap().iter().map(|n| n.name.as_ptr() as usize).collect();
    assert_eq!(first, second, "nothing changed, so nothing should have been re-read");
}

/* ── staying honest ───────────────────────────────────────────────────────── */

#[test]
fn an_edit_made_outside_the_app_arrives_through_reconcile() {
    // Obsidian, a git pull, a collaborator on the share. `reconcile` is what
    // notices, and it has to carry the change through to here or the compiled
    // prompt would keep quoting a paragraph the user deleted an hour ago.
    let (_dir, mut project, kael_id) = ashfall();
    assert_eq!(project.world_nodes().unwrap().len(), 4);

    let path = project.root().join("nodes/character/kael-vantris.md");
    let text = fs::read_to_string(&path).unwrap().replace("Tall, narrow, hooded", "Broad, plated");
    fs::write(&path, text).unwrap();
    // The stamp is `(mtime, size)`, and the two strings are different lengths,
    // so this is noticed however coarse the filesystem's clock is.
    assert!(project.reconcile().unwrap(), "the file changed");

    let nodes = project.world_nodes().unwrap();
    assert_eq!(
        named(nodes, "Kael Vantris").description.as_ref().and_then(|d| d.text("silhouette")),
        Some("Broad, plated"),
    );
    assert_eq!(nodes.iter().filter(|n| n.id == kael_id).count(), 1, "replaced, not duplicated");
}

#[test]
fn a_deleted_node_leaves_the_world_and_a_new_one_joins_it_in_order() {
    let (_dir, mut project, _) = ashfall();
    let guild = named(project.world_nodes().unwrap(), "Cinder Guild").id;

    project.delete_node(guild).unwrap();
    let nodes = project.world_nodes().unwrap();
    assert_eq!(nodes.len(), 3);
    assert!(!nodes.iter().any(|n| n.id == guild));
    // The link that pointed at it goes with it — `delete_node` strips inbound
    // edges, and a world still carrying one would resolve an empty layer card.
    assert!(named(nodes, "Kael Vantris").links.is_empty());

    let lantern = project.create_node(NodeKind::Prop, "Ash Lantern", None).unwrap();
    let ids: Vec<Id> = project.world_nodes().unwrap().iter().map(|n| n.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "a node created later still lands in id order");
    assert!(ids.contains(&lantern.id));
}

#[test]
fn a_file_that_broke_keeps_its_last_good_version_in_the_world() {
    // A sync client copying a half-written file is the expected cause, and the
    // index deliberately keeps the last good row so the entity stays in the
    // navigator. The world has to agree with the navigator: an Inspector that
    // emptied a layer card because somebody else's Dropbox was mid-write would
    // be reporting data loss that has not happened.
    let (_dir, mut project, _) = ashfall();
    let path = project.root().join("nodes/character/kael-vantris.md");
    fs::write(&path, "---\nnot: [valid\n").unwrap();
    project.reconcile().unwrap();

    assert_eq!(project.corrupt_files().unwrap().len(), 1);
    let nodes = project.world_nodes().unwrap();
    assert_eq!(
        named(nodes, "Kael Vantris").description.as_ref().and_then(|d| d.text("silhouette")),
        Some("Tall, narrow, hooded"),
    );
}

#[test]
fn a_rebuilt_index_rebuilds_the_world_rather_than_emptying_it() {
    // `index_rebuild` is offered to the user as a support action. It clears every
    // table, so a cache that only listened for individual row changes would come
    // back holding a world that no longer exists — and the panel would show a
    // stack of nodes the navigator does not.
    let (_dir, mut project, _) = ashfall();
    assert_eq!(project.world_nodes().unwrap().len(), 4);

    project.rebuild_index().unwrap();
    let nodes = project.world_nodes().unwrap();
    assert_eq!(nodes.len(), 4);
    assert_eq!(
        named(nodes, "Kael Vantris").description.as_ref().and_then(|d| d.text("silhouette")),
        Some("Tall, narrow, hooded"),
    );
}

#[test]
fn a_reopened_project_has_a_world_without_being_asked_to_rescan() {
    // The warm open: the index is already populated, so `open` only reconciles
    // and re-reads nothing. The world still has to be there — filling it by
    // reading every node file on open would make every open as slow as the
    // first, which is the cost this whole arrangement exists to avoid.
    let (dir, mut project, _) = ashfall();
    let root = project.root().to_path_buf();
    let before: Vec<Id> = project.world_nodes().unwrap().iter().map(|n| n.id).collect();
    drop(project);

    let mut reopened = Project::open(&root).unwrap();
    let after: Vec<Id> = reopened.world_nodes().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(after, before);
    drop(dir);
}

#[test]
fn a_second_project_is_never_served_the_first_ones_world() {
    // The cache is a field of the open project, which is what makes this
    // impossible rather than merely unlikely: closing a project drops it, and
    // opening another builds a new one against a different index.
    let (_first_dir, mut first, _) = ashfall();
    assert_eq!(first.world_nodes().unwrap().len(), 4);

    let second_dir = tempfile::tempdir().unwrap();
    let mut second = Project::create(second_dir.path(), "Sundering").unwrap();
    let names: Vec<&str> = second.world_nodes().unwrap().iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names.len(), 2, "the two singletons and nothing else");
    assert!(!names.contains(&"Kael Vantris"), "got {names:?}");
}

/* ── the bound the panel is built on ──────────────────────────────────────── */

#[test]
fn a_large_world_is_materialised_once_and_then_costs_nothing_to_ask_for() {
    // `prompt_compile` runs on every Inspector interaction and the engine itself
    // is sub-millisecond by construction. This is the other half of that number:
    // if handing it the world cost a query over every node, the panel would lag
    // and React would be blamed for it.
    //
    // Fastest of several runs, like the bounds in `wobu-influence`'s tests — a
    // single sample measures the machine's load as much as this code. The
    // regression worth catching is a full rebuild creeping back into the steady
    // state, which is three orders of magnitude and fails the fastest run too.
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    for i in 0..300 {
        // Built and saved in one write rather than created and then edited: this
        // is a fixture, and a thousand guarded writes would make the test slower
        // than the thing it is measuring.
        let mut node = Node::new(NodeKind::Setting, format!("District {i}")).unwrap();
        node.description = Some(Description::from_sections([(
            "architecture".to_string(),
            SectionValue::Text("Ash-choked terraces over a basalt shelf".repeat(8)),
        )]));
        saved(project.save_node(node).unwrap());
    }

    let started = std::time::Instant::now();
    assert_eq!(project.world_nodes().unwrap().len(), 302);
    let first = started.elapsed();

    let mut fastest = std::time::Duration::MAX;
    for _ in 0..5 {
        let started = std::time::Instant::now();
        let nodes = project.world_nodes().unwrap();
        fastest = fastest.min(started.elapsed());
        assert_eq!(nodes.len(), 302);
    }

    // The steady state is a lookup, not a query. Generous by two orders of
    // magnitude on purpose: this is a smoke bound against the rebuild coming
    // back, not a microbenchmark, and it runs on whatever CI happens to be.
    assert!(fastest < std::time::Duration::from_micros(50), "steady state took {fastest:?}");
    // And the one build that does happen is a local SQLite read, not five
    // hundred files over a share.
    assert!(first < std::time::Duration::from_millis(500), "first build took {first:?}");
}
