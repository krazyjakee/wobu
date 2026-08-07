//! The index, exercised through the queries the app actually runs.

use super::*;
use crate::atomic::Stamp;
use chrono::Utc;
use std::collections::BTreeSet;
use wobu_core::Link;
use wobu_core::asset::AssetRef;
use wobu_core::{Asset, AssetKind, AssetRole, LinkRole, Node, NodeKind};

fn stamp() -> Stamp {
    Stamp::of_bytes(b"x", 1)
}

fn indexed(index: &Index, node: &Node) {
    let rel = format!("nodes/{}/{}.md", node.kind.dir(), node.slug);
    index.upsert_node(node, &rel, &stamp()).unwrap();
}

#[test]
fn round_trips_a_node_summary() {
    let index = Index::in_memory().unwrap();
    let mut node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    node.summary = "Ex-guild enforcer".into();
    indexed(&index, &node);

    let list = index.list_nodes().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, node.id);
    assert_eq!(list[0].name, "Kael Vantris");
    assert_eq!(list[0].summary, "Ex-guild enforcer");
}

#[test]
fn upserting_the_same_node_twice_does_not_duplicate_it() {
    let index = Index::in_memory().unwrap();
    let mut node = Node::new(NodeKind::Species, "Vashk").unwrap();
    indexed(&index, &node);
    node.name = "Vashk (revised)".into();
    indexed(&index, &node);

    let list = index.list_nodes().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Vashk (revised)");
}

#[test]
fn normal_node_save_is_one_atomic_transaction() {
    let index = Index::in_memory().unwrap();
    let mut node = Node::new(NodeKind::Character, "Before").unwrap();
    indexed(&index, &node);
    let _ = index.take_touched();
    index.reset_write_metrics();

    // Fail after the node row has been replaced. Without one transaction,
    // the new name would survive even though its relationship did not.
    index
        .conn
        .execute_batch(
            "CREATE TRIGGER reject_test_link BEFORE INSERT ON links
             BEGIN SELECT RAISE(ABORT, 'injected link failure'); END;",
        )
        .unwrap();
    node.name = "After".into();
    node.links.push(Link::new(wobu_core::new_id(), LinkRole::MemberOf));

    assert!(index.upsert_node(&node, "nodes/characters/before.md", &stamp()).is_err());
    assert_eq!(index.node(node.id).unwrap().unwrap().name, "Before");
    assert!(index.links().unwrap().is_empty());
    assert_eq!(index.write_metrics.commits.get(), 0);
    assert_eq!(index.write_metrics.preparations.get(), NODE_WRITE_STATEMENT_COUNT);
    match index.take_touched() {
        Touched::These(ids) => assert!(ids.is_empty(), "a rolled-back save was not touched"),
        Touched::Everything => panic!("a rolled-back save marked the whole index"),
    }

    index.conn.execute_batch("DROP TRIGGER reject_test_link").unwrap();
    index.reset_write_metrics();
    index.upsert_node(&node, "nodes/characters/before.md", &stamp()).unwrap();
    assert_eq!(index.write_metrics.commits.get(), 1);
    assert_eq!(index.write_metrics.preparations.get(), NODE_WRITE_STATEMENT_COUNT);
}

#[test]
fn four_thousand_node_rebuild_has_constant_transaction_and_prepare_counts() {
    let index = Index::in_memory().unwrap();
    let asset_id = wobu_core::new_id();

    let make_records = |count: usize| {
        (0..count)
            .map(|number| {
                let mut node =
                    Node::new(NodeKind::Character, format!("Character {number:04}")).unwrap();
                node.links.push(Link::new(wobu_core::new_id(), LinkRole::MemberOf));
                node.asset_links.push(AssetRef::new(asset_id, AssetRole::Pose));
                (node, format!("nodes/characters/character-{number:04}.md"), stamp())
            })
            .collect::<Vec<_>>()
    };

    index.reset_write_metrics();
    index.rebuild_from_scan(&[], &[], &make_records(1), &[]).unwrap();
    let one = (index.write_metrics.commits.get(), index.write_metrics.preparations.get());

    index.reset_write_metrics();
    index.rebuild_from_scan(&[], &[], &make_records(4_000), &[]).unwrap();
    let four_thousand = (index.write_metrics.commits.get(), index.write_metrics.preparations.get());

    assert_eq!(one, (1, REBUILD_STATEMENT_COUNT));
    assert_eq!(four_thousand, one, "batch size must not add commits or prepares");
    assert_eq!(index.list_nodes().unwrap().len(), 4_000);
    assert_eq!(index.links().unwrap().len(), 4_000);
    let asset_link_count: i64 =
        index.conn.query_row("SELECT COUNT(*) FROM asset_links", [], |row| row.get(0)).unwrap();
    assert_eq!(asset_link_count, 4_000);
}

#[test]
fn failed_bulk_rebuild_restores_the_previous_complete_index() {
    let index = Index::in_memory().unwrap();
    let original = Node::new(NodeKind::Setting, "Still Here").unwrap();
    indexed(&index, &original);
    let _ = index.take_touched();
    index.reset_write_metrics();
    index
        .conn
        .execute_batch(
            "CREATE TRIGGER reject_test_node BEFORE INSERT ON nodes
             WHEN NEW.name = 'Break Rebuild'
             BEGIN SELECT RAISE(ABORT, 'injected rebuild failure'); END;",
        )
        .unwrap();

    let broken = Node::new(NodeKind::Setting, "Break Rebuild").unwrap();
    let records = vec![(broken, "nodes/settings/break-rebuild.md".into(), stamp())];
    assert!(index.rebuild_from_scan(&[], &[], &records, &[]).is_err());

    assert_eq!(index.list_nodes().unwrap()[0].id, original.id);
    assert_eq!(index.write_metrics.commits.get(), 0);
    match index.take_touched() {
        Touched::These(ids) => assert!(ids.is_empty(), "a rolled-back rebuild was not touched"),
        Touched::Everything => panic!("a rolled-back rebuild marked the whole index"),
    }
}

#[test]
fn list_is_ordered_by_registry_then_name() {
    let index = Index::in_memory().unwrap();
    for (kind, name) in [
        (NodeKind::Character, "Zara"),
        (NodeKind::Species, "Vashk"),
        (NodeKind::Character, "Aldo"),
        (NodeKind::StyleGuide, "Art Style"),
    ] {
        indexed(&index, &Node::new(kind, name).unwrap());
    }
    let names: Vec<_> = index.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
    assert_eq!(names, ["Art Style", "Vashk", "Aldo", "Zara"]);
}

#[test]
fn links_are_replaced_not_accumulated() {
    let index = Index::in_memory().unwrap();
    let target = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.links.push(Link::new(target, LinkRole::MemberOf));
    indexed(&index, &node);
    indexed(&index, &node);

    assert_eq!(index.backlinks(target).unwrap().len(), 1);
}

#[test]
fn backlinks_answer_who_inherits_from_this() {
    let index = Index::in_memory().unwrap();
    let species = Node::new(NodeKind::Species, "Vashk").unwrap();
    indexed(&index, &species);
    for name in ["Kael", "Oru", "Tam"] {
        let mut c = Node::new(NodeKind::Character, name).unwrap();
        c.links.push(Link::new(species.id, LinkRole::SpeciesOf));
        indexed(&index, &c);
    }
    let back = index.backlinks(species.id).unwrap();
    assert_eq!(back.len(), 3);
    assert!(back.iter().all(|e| e.role == LinkRole::SpeciesOf));
}

#[test]
fn links_answer_the_whole_relationship_map_in_one_read() {
    let index = Index::in_memory().unwrap();
    let species = Node::new(NodeKind::Species, "Vashk").unwrap();
    let culture = Node::new(NodeKind::Culture, "Ember Guild").unwrap();
    let mut character = Node::new(NodeKind::Character, "Kael").unwrap();
    character.links.push(Link::new(species.id, LinkRole::SpeciesOf));
    character.links.push(Link::new(culture.id, LinkRole::MemberOf));
    for node in [&species, &culture, &character] {
        indexed(&index, node);
    }

    let links = index.links().unwrap();
    assert_eq!(links.len(), 2);
    assert!(links.iter().all(|edge| edge.from_id == character.id));
    assert!(links.iter().any(|edge| edge.to_id == species.id));
    assert!(links.iter().any(|edge| edge.to_id == culture.id));
}

#[test]
fn removing_a_node_takes_its_links_and_fts_row_with_it() {
    let index = Index::in_memory().unwrap();
    let target = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    node.links.push(Link::new(target, LinkRole::MemberOf));
    indexed(&index, &node);

    index.remove_node(node.id).unwrap();
    assert!(index.list_nodes().unwrap().is_empty());
    assert!(index.backlinks(target).unwrap().is_empty());
    assert!(index.search("Kael").unwrap().is_empty());
}

#[test]
fn asset_links_are_queryable_by_role_without_reading_a_node_file() {
    // The property M5's per-role image budget depends on: it asks for one
    // role at a time, on every layer of a stack, while compiling.
    let index = Index::in_memory().unwrap();
    let pose = wobu_core::new_id();
    let palette = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.asset_links = vec![
        AssetRef { weight: 0.4, ..AssetRef::new(pose, AssetRole::Pose) },
        AssetRef { weight: 0.9, ..AssetRef::new(palette, AssetRole::Palette) },
        AssetRef::new(palette, AssetRole::Mood),
    ];
    indexed(&index, &node);

    let poses = index.asset_links_in_role(node.id, AssetRole::Pose).unwrap();
    assert_eq!(poses.len(), 1);
    assert_eq!(poses[0].asset_id, pose);
    assert_eq!(poses[0].node_id, node.id, "the index form carries both endpoints");
    assert!(index.asset_links_in_role(node.id, AssetRole::Costume).unwrap().is_empty());

    // Strongest first, because every caller is filling a budget.
    let weights: Vec<f32> =
        index.asset_links_of(node.id).unwrap().iter().map(|l| l.weight).collect();
    assert_eq!(weights, [1.0, 0.9, 0.4]);
}

#[test]
fn one_asset_in_two_roles_is_two_links() {
    // A picture can be both the reference that locks a look and the source
    // of a palette. Keying on the asset alone would silently drop one, and
    // with it one of the two adapters it was meant to reach.
    let index = Index::in_memory().unwrap();
    let asset = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.asset_links =
        vec![AssetRef::new(asset, AssetRole::FullRef), AssetRef::new(asset, AssetRole::Palette)];
    indexed(&index, &node);

    assert_eq!(index.asset_links_of(node.id).unwrap().len(), 2);
    assert_eq!(index.asset_backlinks(asset).unwrap().len(), 2);
}

#[test]
fn asset_links_are_replaced_not_accumulated() {
    // Reconcile re-upserts a node on every external edit, so a merge rather
    // than a replace would make a hand-removed reference immortal.
    let index = Index::in_memory().unwrap();
    let asset = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.asset_links.push(AssetRef::new(asset, AssetRole::Mood));
    indexed(&index, &node);
    indexed(&index, &node);
    assert_eq!(index.asset_backlinks(asset).unwrap().len(), 1);

    node.asset_links.clear();
    indexed(&index, &node);
    assert!(index.asset_links_of(node.id).unwrap().is_empty());
}

#[test]
fn removing_a_node_drops_its_asset_links_but_never_the_asset() {
    // Content-addressed blobs are shared between nodes, so a node going
    // away says nothing about whether the picture is still wanted.
    let index = Index::in_memory().unwrap();
    let asset = Asset {
        id: wobu_core::new_id(),
        hash: "a3".repeat(32),
        kind: AssetKind::Reference,
        rel_path: "assets/originals/a3/a3.png".into(),
        thumb_path: None,
        mime: "image/png".into(),
        width: 8,
        height: 8,
        bytes: 12,
        created_at: Utc::now(),
    };
    index.upsert_asset(&asset).unwrap();

    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.asset_links.push(AssetRef::new(asset.id, AssetRole::Pose));
    indexed(&index, &node);

    index.remove_node(node.id).unwrap();
    assert!(index.asset_backlinks(asset.id).unwrap().is_empty());
    assert!(index.asset(asset.id).unwrap().is_some(), "the blob record must survive");
}

#[test]
fn a_cover_is_readable_without_opening_the_node() {
    let index = Index::in_memory().unwrap();
    let cover = wobu_core::new_id();
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    indexed(&index, &node);
    assert_eq!(index.cover_asset_of(node.id).unwrap(), None);

    node.cover_asset_id = Some(cover);
    indexed(&index, &node);
    assert_eq!(index.cover_asset_of(node.id).unwrap(), Some(cover));

    // Clearing it has to write the null back, not leave the old value —
    // otherwise a removed cover keeps rendering until the next rebuild.
    node.cover_asset_id = None;
    indexed(&index, &node);
    assert_eq!(index.cover_asset_of(node.id).unwrap(), None);
}

#[test]
fn search_covers_notes_and_descriptions_not_just_names() {
    let index = Index::in_memory().unwrap();
    let mut node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    node.notes_raw = "scarred ex-guild enforcer".into();
    indexed(&index, &node);

    assert_eq!(index.search("scarred").unwrap(), vec![node.id]);
    assert_eq!(index.search("Kael").unwrap(), vec![node.id]);
    assert!(index.search("dragon").unwrap().is_empty());
}

#[test]
fn separate_words_do_not_have_to_be_adjacent() {
    // Typing two words you remember from someone's notes is the whole point
    // of searching notes. Wrapping the query as a single quoted phrase makes
    // it an adjacency test instead, which fails on the most natural query a
    // person types.
    let index = Index::in_memory().unwrap();
    let mut node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    node.notes_raw = "scarred ex-guild enforcer".into();
    indexed(&index, &node);

    assert_eq!(index.search("scarred enforcer").unwrap(), vec![node.id]);
    // Across fields, too: one word from the name, one from the notes.
    assert_eq!(index.search("kael scarred").unwrap(), vec![node.id]);
    // Still an AND, not an OR — every word has to appear somewhere.
    assert!(index.search("scarred dragon").unwrap().is_empty());
}

#[test]
fn search_survives_fts_operator_characters() {
    // The filter box searches on every keystroke, so a stray quote or dash
    // must not become a SQL error.
    let index = Index::in_memory().unwrap();
    let node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    indexed(&index, &node);
    for query in ["\"", "-", "*", "ka*", "a AND", "NEAR(", "", "^", "()", "OR OR"] {
        index.search(query).expect(query);
    }

    // Not erroring is the floor, not the bar. A query of pure punctuation
    // has to find nothing rather than everything — dropping the terms and
    // running an empty MATCH would return the whole world.
    for query in ["-", "*", "\"", "()", "  "] {
        assert!(index.search(query).unwrap().is_empty(), "{query} matched something");
    }

    // And the operators must be inert rather than merely safe: these are
    // searches for the literal text, and the node does not contain it.
    assert!(index.search("Kael AND dragon").unwrap().is_empty());
    assert!(index.search("dragon OR Kael").unwrap().is_empty());
    // A stray quote mid-word still finds what the user was reaching for.
    assert_eq!(index.search("Kael\"").unwrap(), vec![node.id]);
}

#[test]
fn kind_and_parent_supports_cycle_checks() {
    let index = Index::in_memory().unwrap();
    let region = Node::new(NodeKind::Setting, "Ember Coast").unwrap();
    let mut city = Node::new(NodeKind::Setting, "Cinder Bay").unwrap();
    city.parent_id = Some(region.id);
    indexed(&index, &region);
    indexed(&index, &city);

    assert_eq!(index.kind_and_parent(city.id).unwrap(), Some((NodeKind::Setting, Some(region.id))));
    assert_eq!(index.kind_and_parent(region.id).unwrap(), Some((NodeKind::Setting, None)));
    assert_eq!(index.children_of(region.id).unwrap(), vec![city.id]);
}

#[test]
fn a_source_version_moves_with_the_description_and_the_edges() {
    // The two inputs a downstream description was built from. A version
    // that missed either would leave stale descriptions reading as current
    // forever, with nothing in the UI to say why.
    let mut vashk = Node::new(NodeKind::Species, "Vashk").unwrap();
    let before = source_version(&vashk);

    vashk.description = Some(wobu_core::Description::from_sections([(
        "anatomy".to_string(),
        wobu_core::SectionValue::Text("Four-jointed legs.".into()),
    )]));
    let described = source_version(&vashk);
    assert_ne!(described, before, "a rewritten description is a new version");

    vashk.links.push(Link::new(wobu_core::new_id(), LinkRole::LocatedIn));
    assert_ne!(source_version(&vashk), described, "a new edge is a new stack");
}

#[test]
fn a_source_version_ignores_labels_and_slider_positions() {
    // Every one of these would otherwise mark a hundred descriptions stale
    // for a change that could not have altered a word of any of them, which
    // is how a signal becomes noise people learn to click past.
    let mut vashk = Node::new(NodeKind::Species, "Vashk").unwrap();
    vashk.links.push(Link::new(wobu_core::new_id(), LinkRole::LocatedIn));
    let before = source_version(&vashk);

    vashk.name = "Vashk (revised)".into();
    vashk.summary = "Ash-adapted".into();
    vashk.notes_raw = "notes are not read from a source".into();
    vashk.links[0].weight = 0.2;
    vashk.touch();
    assert_eq!(source_version(&vashk), before);
}

#[test]
fn reordering_links_by_hand_is_not_a_change() {
    // Somebody tidying the `links:` block in Obsidian has changed nothing
    // the enhance context would see, and must not invalidate the world.
    let (a, b) = (wobu_core::new_id(), wobu_core::new_id());
    let mut node = Node::new(NodeKind::Character, "Kael").unwrap();
    node.links = vec![Link::new(a, LinkRole::MemberOf), Link::new(b, LinkRole::LocatedIn)];
    let before = source_version(&node);

    node.links.reverse();
    assert_eq!(source_version(&node), before);
}

#[test]
fn a_subject_version_leaves_out_the_description_it_produced() {
    // The subject's description is the *output* of an enhance. Folding it
    // in would make every description stale the moment it was written, and
    // would report a hand-edit as staleness rather than as the resolution
    // it is.
    let mut kael = Node::new(NodeKind::Character, "Kael").unwrap();
    kael.notes_raw = "scarred, ex-guild".into();
    let before = subject_version(&kael);

    kael.description = Some(wobu_core::Description::from_sections([(
        "silhouette".to_string(),
        wobu_core::SectionValue::Text("Tall.".into()),
    )]));
    assert_eq!(subject_version(&kael), before);

    kael.notes_raw.push_str("\nowes a debt");
    assert_ne!(subject_version(&kael), before, "notes are the subject's own input");
}

#[test]
fn a_cycle_does_not_hang_the_downstream_walk() {
    // Two nodes each claiming the other is upstream is two clicks away in
    // the Relations panel, and one line away in Obsidian. First visit wins,
    // exactly as it does in `wobu_influence::resolve`.
    let index = Index::in_memory().unwrap();
    let mut a = Node::new(NodeKind::Culture, "Ember Guild").unwrap();
    let mut b = Node::new(NodeKind::Culture, "Ash Court").unwrap();
    a.links.push(Link::new(b.id, LinkRole::MemberOf));
    b.links.push(Link::new(a.id, LinkRole::MemberOf));
    indexed(&index, &a);
    indexed(&index, &b);

    assert_eq!(index.dependents_of(a.id).unwrap(), BTreeSet::from([b.id]));
    assert_eq!(index.dependents_of(b.id).unwrap(), BTreeSet::from([a.id]));
}

#[test]
fn a_lateral_link_is_a_source_but_not_a_route() {
    // `related_to` resolves to a source and is never expanded through, so
    // downstream it reaches exactly one hop. Walking further would make a
    // nod at a sibling drag that sibling's whole ancestry into the
    // invalidation, and every character in a world would depend on every
    // other one.
    let index = Index::in_memory().unwrap();
    let far = Node::new(NodeKind::Culture, "Ash Court").unwrap();
    let mut middle = Node::new(NodeKind::Culture, "Ember Guild").unwrap();
    middle.links.push(Link::new(far.id, LinkRole::RelatedTo));
    let mut kael = Node::new(NodeKind::Character, "Kael").unwrap();
    kael.links.push(Link::new(middle.id, LinkRole::RelatedTo));
    for node in [&far, &middle, &kael] {
        indexed(&index, node);
    }

    // Kael nods at the Guild, so the Guild's change reaches him.
    assert!(index.dependents_of(middle.id).unwrap().contains(&kael.id));
    // The Guild nods at the Court, and that is where it stops: Kael never
    // expands the Guild, so the Court is not in his stack.
    assert_eq!(index.dependents_of(far.id).unwrap(), BTreeSet::from([middle.id]));
}

#[test]
fn everything_is_downstream_of_the_style_guide() {
    // Nothing links to it — it is a root of every stack — so a walk over
    // the edges would find nobody and report that editing the one node
    // which governs the whole project changed nothing at all.
    let index = Index::in_memory().unwrap();
    let style = Node::new(NodeKind::StyleGuide, "Style Guide").unwrap();
    let vashk = Node::new(NodeKind::Species, "Vashk").unwrap();
    let kael = Node::new(NodeKind::Character, "Kael").unwrap();
    for node in [&style, &vashk, &kael] {
        indexed(&index, node);
    }

    assert_eq!(index.dependents_of(style.id).unwrap(), BTreeSet::from([vashk.id, kael.id]));
}

#[test]
fn a_version_bump_rebuilds_rather_than_migrating() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.sqlite");
    {
        let index = Index::open_at(&path).unwrap();
        indexed(&index, &Node::new(NodeKind::Species, "Vashk").unwrap());
        assert!(!index.is_empty().unwrap());
        index
            .conn
            .execute("UPDATE meta SET value = '999' WHERE key = 'index_version'", [])
            .unwrap();
    }
    let reopened = Index::open_at(&path).unwrap();
    assert!(reopened.is_empty().unwrap(), "stale schema should be discarded");
}

#[test]
fn all_stamps_keys_by_relative_path() {
    let index = Index::in_memory().unwrap();
    let node = Node::new(NodeKind::Species, "Vashk").unwrap();
    indexed(&index, &node);
    let stamps = index.all_stamps().unwrap();
    assert_eq!(stamps.len(), 1);
    assert!(stamps.contains_key("nodes/species/vashk.md"));
}
