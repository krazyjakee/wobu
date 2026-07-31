//! When a description stops being current, and what that costs.
//!
//! Wobu treats `description_state = stale` as **derived**, not stored. Four of
//! the five states are facts about what somebody did and live in the Markdown;
//! "is this still current" is a question about the rest of the world, and the
//! answer is computed from an `enhanced_from` stamp against the index.
//!
//! That choice is what this file exists to hold in place, because the
//! alternative fails in a way nobody would notice until it hurt. Editing the
//! Style Guide invalidates most of a project. If staleness were a stored field
//! it would have to be written into every affected node's frontmatter: a
//! hundred guarded writes over a share, a hundred chances to lose a race with a
//! collaborator and park a conflict sibling, and a hundred files whose
//! `updated_at` moved for a change the user did not make to them. Worse, the
//! enum holds one value at a time, so writing `stale` over `edited` would
//! forget that a person wrote those words by hand — and the next enhance would
//! overwrite them without knowing to ask.

use std::collections::HashMap;
use std::path::PathBuf;

use wobu_core::{Description, DescriptionState, Id, Link, LinkRole, NodeKind, SectionValue};
use wobu_store::project::Enhanced;
use wobu_store::{Project, SaveOutcome};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A world with the shape the layering was designed for: two singleton roots, a
/// species and a culture beneath them, and two characters held to those.
struct World {
    dir: tempfile::TempDir,
    project: Project,
    style: Id,
    bible: Id,
    vashk: Id,
    guild: Id,
    kael: Id,
}

fn ashfall() -> World {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();

    let singleton = |project: &Project, kind| project.index().singleton_of(kind).unwrap().unwrap();
    let style = singleton(&project, NodeKind::StyleGuide);
    let bible = singleton(&project, NodeKind::WorldBible);

    let vashk = project.create_node(NodeKind::Species, "Vashk", None).unwrap().id;
    let guild = project.create_node(NodeKind::Culture, "Cinder Guild", None).unwrap().id;

    let mut kael = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    kael.links = vec![Link::new(vashk, LinkRole::SpeciesOf), Link::new(guild, LinkRole::MemberOf)];
    let kael = saved(&mut project, kael);

    let mut oru = project.create_node(NodeKind::Character, "Oru", None).unwrap();
    oru.links = vec![Link::new(vashk, LinkRole::SpeciesOf)];
    let oru = saved(&mut project, oru);

    // Enhance everything, stamping the stack each node was built from — which
    // is what `wobu_influence::resolve` hands back for it. Nothing is stale
    // until something moves.
    for (id, sources) in [
        (style, vec![]),
        (bible, vec![style]),
        (vashk, vec![style, bible]),
        (guild, vec![style, bible]),
        (kael, vec![style, bible, vashk, guild]),
        (oru, vec![style, bible, vashk]),
    ] {
        enhance(&mut project, id, "Modern firearms", &sources);
    }
    assert!(stale(&project).is_empty(), "the fixture should start current");

    World { dir, project, style, bible, vashk, guild, kael }
}

/// Every kind declares `never`, so this is the one section that can stand in
/// for a description of anything.
fn description(text: &str) -> Description {
    Description::from_sections([("never".to_string(), SectionValue::List(vec![text.to_string()]))])
}

fn enhance(project: &mut Project, id: Id, text: &str, sources: &[Id]) {
    match project.accept_enhanced(id, description(text), sources, false).unwrap() {
        Enhanced::Saved(_) => {}
        other => panic!("expected a clean enhance, got {other:?}"),
    }
}

fn saved(project: &mut Project, node: wobu_core::Node) -> Id {
    match project.save_node(node).unwrap() {
        SaveOutcome::Saved(node) => node.id,
        SaveOutcome::Conflict { conflict_path } => panic!("unexpected conflict at {conflict_path}"),
    }
}

/// The names the navigator would put a dot beside.
fn stale(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = project
        .list_nodes()
        .unwrap()
        .into_iter()
        .filter(|n| n.description_state == DescriptionState::Stale)
        .map(|n| n.name)
        .collect();
    names.sort();
    names
}

/// The bytes and modification time of every node file, keyed by path.
fn snapshot(project: &Project) -> HashMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut out = HashMap::new();
    for entry in walkdir::WalkDir::new(project.root().join("nodes")).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let meta = entry.metadata().unwrap();
        out.insert(
            entry.path().to_path_buf(),
            (std::fs::read(entry.path()).unwrap(), meta.modified().unwrap()),
        );
    }
    out
}

fn path_of(project: &Project, id: Id) -> PathBuf {
    let rel = project.index().rel_path_of(id).unwrap().expect("indexed");
    wobu_store::paths::from_rel_string(project.root(), &rel)
}

/* ── the crux: invalidating the world writes one file ─────────────────────── */

#[test]
fn re_enhancing_the_style_guide_marks_the_world_stale_and_writes_only_its_own_file() {
    // The whole issue in one test. The Style Guide is layer 1 of every stack,
    // so rewriting its description invalidates every description in the
    // project. Storing that answer would mean a write per node; deriving it
    // means a write for the node the user actually edited and nothing else.
    let mut w = ashfall();
    let before = snapshot(&w.project);

    enhance(&mut w.project, w.style, "Symmetrical faces", &[]);

    assert_eq!(
        stale(&w.project),
        ["Cinder Guild", "Kael Vantris", "Oru", "Vashk", "World Canon"],
        "everything downstream of the style should be offered a re-enhance",
    );

    let style_path = path_of(&w.project, w.style);
    let after = snapshot(&w.project);
    assert_eq!(after.len(), before.len(), "no file should have appeared or gone");
    for (path, (bytes, mtime)) in &before {
        if path == &style_path {
            continue;
        }
        let (now_bytes, now_mtime) = after.get(path).expect("a node file vanished");
        assert_eq!(now_bytes, bytes, "{} was rewritten", path.display());
        assert_eq!(now_mtime, mtime, "{}'s updated_at moved", path.display());
    }
}

#[test]
fn a_re_enhance_clears_the_dot_for_that_node_alone() {
    // Resolving one node's staleness must not silently resolve anyone else's —
    // the others' descriptions still came from the old style.
    let mut w = ashfall();
    enhance(&mut w.project, w.style, "Symmetrical faces", &[]);

    enhance(&mut w.project, w.kael, "Modern firearms", &[w.style, w.bible, w.vashk, w.guild]);
    assert_eq!(stale(&w.project), ["Cinder Guild", "Oru", "Vashk", "World Canon"]);
}

/* ── what counts as a change ──────────────────────────────────────────────── */

#[test]
fn editing_a_nodes_own_notes_makes_only_that_node_stale() {
    // Notes are the subject's own half of the enhance context and reach nobody
    // else: the context is built from the stack's *descriptions*, not their raw
    // notes. Marking the world stale for a typo in one node's scratchpad is the
    // noise that teaches people to ignore the dot.
    let mut w = ashfall();
    let mut vashk = w.project.get_node(w.vashk).unwrap();
    vashk.notes_raw = "ash-adapted, subterranean".into();
    saved(&mut w.project, vashk);

    assert_eq!(stale(&w.project), ["Vashk"]);
}

#[test]
fn a_new_edge_on_an_intermediate_node_reaches_the_far_side_of_the_stack() {
    // A culture that gains a place puts a whole place chain in front of every
    // character in it, so their descriptions were built from a smaller stack
    // than a re-enhance would see — even though not one description changed.
    // This is why an enhance stamp records its sources' *edges* and not only
    // their descriptions, and it is what lets staleness be answered without a
    // walk.
    let mut w = ashfall();
    let bay = w.project.create_node(NodeKind::Setting, "Cinder Bay", None).unwrap();

    let mut guild = w.project.get_node(w.guild).unwrap();
    guild.links.push(Link::new(bay.id, LinkRole::LocatedIn));
    saved(&mut w.project, guild);

    assert_eq!(
        stale(&w.project),
        ["Cinder Guild", "Kael Vantris"],
        "Kael is a member of the guild and never saw the bay",
    );
}

#[test]
fn renaming_an_upstream_node_invalidates_nothing() {
    // The counterweight to the test above. Names are labels; nothing a renamed
    // species contributes to a character's description changes because somebody
    // fixed its spelling.
    let mut w = ashfall();
    let mut vashk = w.project.get_node(w.vashk).unwrap();
    vashk.name = "Vashk-Prime".into();
    vashk.summary = "Ash-adapted".into();
    saved(&mut w.project, vashk);

    assert!(stale(&w.project).is_empty(), "{:?}", stale(&w.project));
}

#[test]
fn muting_an_influence_edge_makes_its_dependent_stale() {
    // `enabled: false` is the world's own off switch, and the resolver skips it
    // — so a muted edge means the next enhance would see a different stack than
    // the last one did.
    let mut w = ashfall();
    let mut kael = w.project.get_node(w.kael).unwrap();
    kael.links[0].enabled = false;
    saved(&mut w.project, kael);

    assert_eq!(stale(&w.project), ["Kael Vantris"]);
}

#[test]
fn deleting_an_upstream_source_leaves_its_dependents_stale() {
    // The layer that species contributed is simply gone. Reporting the
    // survivors as current would leave descriptions in the world citing a
    // species nobody can open.
    let mut w = ashfall();
    w.project.delete_node(w.vashk).unwrap();

    assert_eq!(stale(&w.project), ["Kael Vantris", "Oru"]);
}

#[test]
fn a_description_typed_straight_into_the_markdown_is_not_called_stale() {
    // Obsidian users, and every description written before stamps existed.
    // There is nothing to compare them against, and a dot on every row of an
    // existing project is exactly the noise this feature must not create.
    let mut w = ashfall();
    let mut node = w.project.create_node(NodeKind::Prop, "Ashglass Lantern", None).unwrap();
    node.description = Some(description("Modern firearms"));
    node.description_state = DescriptionState::Fresh;
    saved(&mut w.project, node);

    enhance(&mut w.project, w.style, "Symmetrical faces", &[]);
    assert!(!stale(&w.project).contains(&"Ashglass Lantern".to_string()));
}

/* ── the data-loss rule ───────────────────────────────────────────────────── */

#[test]
fn a_hand_edited_description_is_never_overwritten_by_a_re_enhance() {
    // The rule the issue states as a rule and not a nicety. A description the
    // user rewrote is the only copy of those words anywhere.
    let mut w = ashfall();
    let mut kael = w.project.get_node(w.kael).unwrap();
    kael.description = Some(description("My own words"));
    kael.description_state = DescriptionState::Edited;
    saved(&mut w.project, kael);

    let outcome = w
        .project
        .accept_enhanced(w.kael, description("The machine's words"), &[w.style], false)
        .unwrap();
    assert!(matches!(outcome, Enhanced::RefusedEdit(_)), "{outcome:?}");

    let on_disk = w.project.get_node(w.kael).unwrap();
    assert_eq!(on_disk.description.unwrap().never(), ["My own words"]);
    assert_eq!(on_disk.description_state, DescriptionState::Edited);
}

#[test]
fn the_user_can_still_choose_to_replace_their_own_words() {
    // Refusing is the default, not the only answer — otherwise a hand-edit
    // would be a one-way door and the node could never be enhanced again.
    let mut w = ashfall();
    let mut kael = w.project.get_node(w.kael).unwrap();
    kael.description = Some(description("My own words"));
    kael.description_state = DescriptionState::Edited;
    saved(&mut w.project, kael);

    let outcome = w
        .project
        .accept_enhanced(w.kael, description("The machine's words"), &[w.style], true)
        .unwrap();
    assert!(matches!(outcome, Enhanced::Saved(_)), "{outcome:?}");

    let on_disk = w.project.get_node(w.kael).unwrap();
    assert_eq!(on_disk.description.unwrap().never(), ["The machine's words"]);
    assert_eq!(on_disk.description_state, DescriptionState::Fresh);
}

#[test]
fn edited_survives_the_node_going_stale() {
    // The reason staleness is derived rather than stored, stated as a test. A
    // stored `stale` would have to overwrite `edited`, and the enhance path
    // would then have nothing left to ask the user about — it would replace
    // their prose believing it had written it itself.
    let mut w = ashfall();
    let mut kael = w.project.get_node(w.kael).unwrap();
    kael.description = Some(description("My own words"));
    kael.description_state = DescriptionState::Edited;
    saved(&mut w.project, kael);

    enhance(&mut w.project, w.vashk, "Symmetrical faces", &[w.style, w.bible]);

    assert!(stale(&w.project).contains(&"Kael Vantris".to_string()), "the dot should appear");
    assert_eq!(
        w.project.get_node(w.kael).unwrap().description_state,
        DescriptionState::Edited,
        "the file must still remember who wrote the description",
    );
    let outcome = w
        .project
        .accept_enhanced(w.kael, description("The machine's words"), &[w.style], false)
        .unwrap();
    assert!(matches!(outcome, Enhanced::RefusedEdit(_)), "{outcome:?}");
}

/* ── the folder is the only copy ──────────────────────────────────────────── */

#[test]
fn staleness_survives_the_index_being_deleted() {
    // The index is disposable and rebuilt from the Markdown, so a stamp that
    // lived only there would come back empty for the whole project on the
    // first rebuild — and every stale description in the world would silently
    // read as current.
    let mut w = ashfall();
    enhance(&mut w.project, w.style, "Symmetrical faces", &[]);
    let before = stale(&w.project);
    assert!(!before.is_empty());

    let root = w.project.root().to_path_buf();
    let index = w.project.index_path();
    drop(w.project);
    std::fs::remove_file(&index).ok();

    let reopened = Project::open(&root).unwrap();
    assert_eq!(stale(&reopened), before);
    drop(w.dir);
}

#[test]
fn a_stamp_is_readable_prose_in_the_frontmatter() {
    // People open these files in Obsidian. A stamp that filled the screen, or
    // that named files by a path, would be the reason somebody deleted the key
    // by hand — and deleting it silently turns staleness off for that node.
    let w = ashfall();
    let text = std::fs::read_to_string(path_of(&w.project, w.kael)).unwrap();
    let stamp = text.split("links:").next().unwrap();

    assert!(stamp.contains("enhanced_from:"), "{text}");
    assert!(stamp.contains("subject:"), "{text}");
    assert!(stamp.contains("sources:"), "{text}");
    assert!(!text.contains(&w.project.root().to_string_lossy().into_owned()), "{text}");
    // Four sources, each two short lines. Anything longer and the notes are
    // below the fold when the file is opened.
    assert!(stamp.lines().count() < 20, "{stamp}");
}

#[test]
fn a_node_reached_twice_is_stamped_once_per_source() {
    // `resolve` hands back a stack with first-visit-wins already applied, so a
    // node reachable by two routes arrives once. Stamping it twice would double
    // the cost of every check for no more information.
    let w = ashfall();
    let stamp = w.project.get_node(w.kael).unwrap().enhanced_from.unwrap();
    let mut sources: Vec<Id> = stamp.sources.iter().map(|s| s.node).collect();
    sources.sort();
    sources.dedup();
    assert_eq!(sources.len(), stamp.sources.len());
    assert!(!sources.contains(&w.kael), "the subject is not a source of itself");
}
