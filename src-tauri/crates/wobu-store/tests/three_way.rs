//! A replica on another machine, folded into this one.
//!
//! `multi_writer.rs` covers the other multi-user shape: one folder, several
//! people, and a compare-and-swap that catches whoever renamed second. This file
//! is the case that CAS provably cannot catch. With a replica per machine both
//! writers succeed against their own copy, no rename ever loses, and the two
//! folders simply hold different bytes — so the only thing that can tell "we
//! both edited it" from "one of us did" is the hash the two of us last agreed
//! on, which is what `sync_state` (#78) is for and what `wobu_store::apply`
//! reads.
//!
//! **How the peer is simulated.** A peer is another machine, so it appears the
//! only way another machine ever can: as an [`Incoming`] payload of Markdown
//! bytes with a node id on it. Driving a second `Project` would be less faithful
//! rather than more — the two would have to be two *different* projects to have
//! two indexes, and then nothing would share a node id, which is the thing under
//! test. Building the bytes with `markdown::to_markdown` is exactly what the
//! peer's own `save_node` wrote before it put them on the wire.
//!
//! Every assertion here is a variation on one question, the same one
//! `multi_writer.rs` asks: after the dust settles, is anybody's text gone?

use std::fs;
use std::path::{Path, PathBuf};

use wobu_core::{Id, Node, NodeKind};
use wobu_store::apply::{Applied, Decision, Incoming, Refused};
use wobu_store::{Project, markdown, paths};

/// Two peers, spelled the way iroh renders an `EndpointId`.
const NADIA: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
const PRIYA: &str = "f9e8d7c6b5a4b3a2918071605f4e3d2c1b0af9e8d7c6b5a4b3a2918071605f4e";

/// The alias each of those ids derives to, and therefore the name that goes into
/// a sibling written on their behalf. Spelled out rather than computed so that a
/// change to the derivation fails here rather than silently renaming every
/// conflict file on every share.
const NADIA_ALIAS: &str = "patient-vale-0a1b";

fn world() -> (tempfile::TempDir, Project, Node, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let path = node_path(&project, node.id);
    assert!(path.is_file(), "the create should have written a file");
    (dir, project, node, path)
}

fn node_path(project: &Project, id: Id) -> PathBuf {
    let rel = project.index().rel_path_of(id).unwrap().expect("indexed");
    paths::from_rel_string(project.root(), &rel)
}

/// The bytes a peer sends: their copy of the node, rendered as their `save_node`
/// would have rendered it.
fn from_peer(node: &Node, notes: &str) -> Incoming {
    let mut theirs = node.clone();
    theirs.notes_raw = notes.into();
    // Their save re-stamped the clock, as every save does. Keeping it identical
    // would make byte equality reachable by accident and hide the case where two
    // machines really do differ.
    theirs.updated_at += chrono::Duration::seconds(30);
    Incoming {
        node_id: theirs.id,
        slug: theirs.slug.clone(),
        text: markdown::to_markdown(&theirs).unwrap(),
    }
}

/// What we last agreed with a peer about a node, as `sync_state` holds it.
fn agree(project: &Project, peer: &str, id: Id) {
    let hash = project.outgoing(id).unwrap().expect("on disk").hash;
    project.record_agreed(peer, &[(id, hash)]).unwrap();
}

/// Everything in the folder, agreed with a peer — the state two machines are in
/// after one clean sync. A fresh project has singleton nodes in it as well as
/// the one a test made, and a manifest is about all of them.
fn agree_all(project: &Project, peer: &str) {
    project.record_agreed(peer, &project.manifest().unwrap()).unwrap();
}

fn base(project: &Project, peer: &str, id: Id) -> Option<String> {
    project.index().base_hash(peer, id).unwrap()
}

fn siblings(path: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(".conflict-"))
        .collect();
    out.sort();
    out
}

/// Our own edit, saved the ordinary way.
fn we_edit(project: &mut Project, id: Id, notes: &str) {
    let mut ours = project.get_node(id).unwrap();
    ours.notes_raw = notes.into();
    project.save_node(ours).unwrap();
}

fn only(report: &wobu_store::ApplyReport) -> &Applied {
    assert_eq!(report.outcomes.len(), 1, "{report:?}");
    &report.outcomes[0].1
}

/* ── the truth table, over a real folder ──────────────────────────────────── */

#[test]
fn we_are_at_the_base_so_their_version_lands_and_the_base_moves() {
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);

    let theirs = from_peer(&node, "nadia rewrote the second act");
    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();

    assert!(matches!(only(&report), Applied::FastForwarded { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs.text, "their bytes are not on disk");
    assert!(siblings(&path).is_empty(), "a fast-forward should not park anything");
    assert!(report.changed_the_folder());

    // The index has to agree with the disk, or the UI shows text that is not
    // there and the user builds on a version nobody else has.
    assert!(project.get_node(node.id).unwrap().notes_raw.contains("nadia rewrote"));

    let moved = base(&project, NADIA, node.id);
    assert_eq!(
        moved.as_deref(),
        Some(wobu_store::atomic::hash_bytes(theirs.text.as_bytes()).as_str())
    );
}

#[test]
fn we_moved_and_they_did_not_so_nothing_is_written_and_they_are_told() {
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);

    // They send back the version we agreed on; we have since edited ours.
    let unchanged = Incoming {
        node_id: node.id,
        slug: node.slug.clone(),
        text: fs::read_to_string(&path).unwrap(),
    };
    we_edit(&mut project, node.id, "our afternoon's work");
    let ours = fs::read_to_string(&path).unwrap();

    let report = project.apply_from_peer(NADIA, &[unchanged]).unwrap();

    assert_eq!(only(&report), &Applied::SendOurs);
    assert_eq!(report.to_send(), vec![node.id], "the caller has to know to push");
    assert_eq!(fs::read_to_string(&path).unwrap(), ours, "our newer version was overwritten");
    assert!(siblings(&path).is_empty());
    assert!(!report.changed_the_folder(), "nothing moved, so nothing should refetch");
}

#[test]
fn the_base_does_not_move_when_we_merely_decide_to_send() {
    // A base is a claim that the *peer* holds these bytes, and the next sync
    // fast-forwards on it without asking anybody. Moving it when we put a node
    // on the wire, rather than when they said it arrived, turns a dropped
    // connection into a base describing a file they never got — and their next
    // edit then reads as one-sided and overwrites ours.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let agreed = base(&project, NADIA, node.id);

    let unchanged = Incoming {
        node_id: node.id,
        slug: node.slug.clone(),
        text: fs::read_to_string(&path).unwrap(),
    };
    we_edit(&mut project, node.id, "ours");
    project.apply_from_peer(NADIA, &[unchanged]).unwrap();

    assert_eq!(base(&project, NADIA, node.id), agreed, "the base ran ahead of the peer");
}

#[test]
fn both_of_us_reaching_the_same_bytes_only_moves_the_base() {
    // Two people typing the same paragraph, or a third machine having already
    // delivered this version to both of us. Nobody's text is at risk, so a
    // conflict card here would be pure noise — and noise is expensive, because
    // it teaches people to dismiss the card that one day matters.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);

    let converged = from_peer(&node, "we both wrote this");
    fs::write(&path, &converged.text).unwrap();
    project.rescan().unwrap();
    let before = fs::metadata(&path).unwrap().modified().unwrap();

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&converged)).unwrap();

    assert_eq!(only(&report), &Applied::Converged);
    assert!(siblings(&path).is_empty(), "identical bytes must not park a sibling");
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before, "the file was rewritten");
    assert_eq!(
        base(&project, NADIA, node.id).as_deref(),
        Some(wobu_store::atomic::hash_bytes(converged.text.as_bytes()).as_str()),
    );
}

#[test]
fn both_of_us_moving_differently_parks_theirs_and_never_touches_ours() {
    // The row the whole milestone is about, and the one `guarded_write` cannot
    // reach on its own: both machines wrote successfully to their own folder, so
    // no compare-and-swap failed anywhere.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);

    we_edit(&mut project, node.id, "our version of the second act");
    let ours = fs::read_to_string(&path).unwrap();
    let theirs = from_peer(&node, "their version of the second act");

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();

    let Applied::Conflicted { conflict_path } = only(&report) else { panic!("{report:?}") };
    assert_eq!(fs::read_to_string(&path).unwrap(), ours, "a peer overwrote a local edit");

    let parked = paths::from_rel_string(project.root(), conflict_path);
    assert_eq!(fs::read_to_string(&parked).unwrap(), theirs.text, "their text is not anywhere");
    assert_eq!(siblings(&path), vec![parked], "exactly one sibling");
    assert!(report.changed_the_folder(), "a new sibling is a change the UI has to see");
}

/* ── the invariant ────────────────────────────────────────────────────────── */

#[test]
fn a_peer_we_have_never_synced_with_can_never_fast_forward() {
    // The single most important test in this file. `base_hash` returning `None`
    // is the ordinary never-synced state, and reading it as "we are at the base"
    // would overwrite this folder with a stranger's copy on first contact —
    // silently, before anybody had a chance to look at it. The failure direction
    // is a conflict card nobody needed, never an overwrite.
    let (_dir, mut project, node, path) = world();
    assert_eq!(base(&project, NADIA, node.id), None, "the test needs a virgin peer");

    // Note that we have *not* edited our copy: as far as this folder knows,
    // nothing has happened since the node was created. That is exactly the state
    // a naive implementation reads as "unchanged, so take theirs".
    let ours = fs::read_to_string(&path).unwrap();
    let theirs = from_peer(&node, "a version this machine has never seen");

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();

    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), ours, "no base must never mean take theirs");
    assert_eq!(siblings(&path).len(), 1);
    assert_eq!(fs::read_to_string(&siblings(&path)[0]).unwrap(), theirs.text);
    assert_eq!(base(&project, NADIA, node.id), None, "a conflict agreed nothing");
}

#[test]
fn a_conflict_sibling_lands_while_the_local_file_is_byte_for_byte_unchanged() {
    // Restated from the other side, because this is the precise shape
    // `guarded_write` gets wrong. Its CAS asks "has this file moved since I read
    // it"; here the answer is no, and the right answer is still "do not write".
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    // Everything the folder can observe says the file is settled: the index
    // stamp matches the disk exactly, and re-saving it would be a clean write.
    let stamp = project.index().stamp_of(node.id).unwrap().unwrap();
    let on_disk = fs::read_to_string(&path).unwrap();
    assert_eq!(stamp.hash, wobu_store::atomic::hash_bytes(on_disk.as_bytes()));

    let report = project.apply_from_peer(NADIA, &[from_peer(&node, "theirs")]).unwrap();

    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), on_disk);
    assert_eq!(
        project.index().stamp_of(node.id).unwrap().unwrap().hash,
        stamp.hash,
        "the index moved for a node whose file did not"
    );
}

#[test]
fn a_fast_forward_that_loses_its_race_parks_theirs_and_keeps_the_local_save() {
    // The window between reading the local file and renaming over it. On a share
    // that window is real: another Wobu, or this user's own autosave landing a
    // millisecond later. The peer's version must lose, because the file it was
    // about to replace is no longer the one we compared.
    //
    // The race is simulated by moving the file after the base was recorded and
    // before `apply` runs, which is the same thing from the write's point of
    // view: `guarded_write` is handed a stamp that no longer describes the disk.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let agreed = base(&project, NADIA, node.id);

    let theirs = from_peer(&node, "theirs, arriving");
    // Our save lands first, so the base still says we are unchanged — the
    // decision is `FastForward` — but the bytes underneath have moved.
    let mut racing = project.get_node(node.id).unwrap();
    racing.notes_raw = "the save that got there first".into();
    racing.updated_at += chrono::Duration::seconds(1);
    let ours = markdown::to_markdown(&racing).unwrap();
    fs::write(&path, &ours).unwrap();

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();

    let Applied::Conflicted { conflict_path } = only(&report) else {
        panic!("a lost race must not report a clean write: {report:?}")
    };
    assert_eq!(fs::read_to_string(&path).unwrap(), ours, "the local save was clobbered");
    let parked = paths::from_rel_string(project.root(), conflict_path);
    assert_eq!(fs::read_to_string(&parked).unwrap(), theirs.text);
    assert_eq!(
        base(&project, NADIA, node.id),
        agreed,
        "a lost race agreed nothing, so the base must not have moved"
    );
}

#[test]
fn the_sibling_is_named_after_the_peer_that_sent_it_and_not_after_us() {
    // The sibling holds *their* paragraph. Stamping it with this installation's
    // alias would put somebody else's writing on a card under our name, and
    // `Conflict::mine` would then offer "keep mine" for text we never wrote.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    project.apply_from_peer(NADIA, &[from_peer(&node, "theirs")]).unwrap();

    let name = siblings(&path)[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(&format!("kael-vantris.conflict-{NADIA_ALIAS}-")), "{name}");

    // And the card reads it back as theirs, which is the whole reason the name
    // is derived from the key rather than assigned.
    let cards = project.conflicts().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].user.as_deref(), Some(NADIA_ALIAS));
    assert!(!cards[0].mine, "their version was attributed to us");
    assert!(cards[0].parked.contains("theirs"));
    assert!(cards[0].current.contains("ours"));
}

#[test]
fn syncing_the_same_disagreement_again_does_not_grow_a_second_sibling() {
    // A conflict moves no base, so the same disagreement is rediscovered on
    // every round until a human resolves it. Without this the folder grows one
    // identical sibling per poll and the user cannot tell the copies apart.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");
    let theirs = from_peer(&node, "theirs");

    let first = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    let second = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    let third = project.apply_from_peer(NADIA, &[theirs]).unwrap();

    assert!(matches!(only(&first), Applied::Conflicted { .. }));
    assert!(matches!(only(&second), Applied::AlreadyParked { .. }), "{second:?}");
    assert!(matches!(only(&third), Applied::AlreadyParked { .. }));
    assert_eq!(siblings(&path).len(), 1, "a poll loop filled the folder: {:?}", siblings(&path));
    assert!(!second.changed_the_folder(), "nothing landed, so nothing should refetch");
    assert!(second.parked().is_empty(), "a card was raised twice for one decision");
}

#[test]
fn a_genuinely_different_second_version_still_parks_beside_the_first() {
    // The guard above must not become "we already have a sibling, so skip". Two
    // different versions from the same peer are two paragraphs and both have to
    // survive.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    project.apply_from_peer(NADIA, &[from_peer(&node, "their first thought")]).unwrap();
    project.apply_from_peer(NADIA, &[from_peer(&node, "their second thought")]).unwrap();

    let parked = siblings(&path);
    assert_eq!(parked.len(), 2, "{parked:?}");
    let bodies: Vec<String> = parked.iter().map(|p| fs::read_to_string(p).unwrap()).collect();
    assert!(bodies.iter().any(|b| b.contains("their first thought")));
    assert!(bodies.iter().any(|b| b.contains("their second thought")));
}

#[test]
fn resolving_a_conflict_in_favour_of_the_peer_settles_the_sync() {
    // The loop has to close. Once the user takes the parked version the two
    // machines hold the same bytes, so the next round reads `Converged`, the
    // base catches up, and nothing is written or asked about again.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");
    let theirs = from_peer(&node, "theirs");
    project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();

    let card = project.conflicts().unwrap().remove(0);
    project.resolve_conflict(&card.rel_path, wobu_store::Keep::Parked, &card.current_hash).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs.text);

    let after = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(only(&after), &Applied::Converged);
    assert_eq!(
        base(&project, NADIA, node.id).as_deref(),
        Some(wobu_store::atomic::hash_bytes(theirs.text.as_bytes()).as_str()),
    );
    assert!(siblings(&path).is_empty());
}

/* ── nodes one side does not have ─────────────────────────────────────────── */

#[test]
fn a_node_made_on_another_machine_arrives_as_a_new_file() {
    let (_dir, mut project, _node, _path) = world();

    let mut theirs = Node::new(NodeKind::Setting, "The Ember Coast").unwrap();
    theirs.notes_raw = "a place only nadia has".into();
    let incoming = Incoming {
        node_id: theirs.id,
        slug: theirs.slug.clone(),
        text: markdown::to_markdown(&theirs).unwrap(),
    };

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&incoming)).unwrap();

    let Applied::FastForwarded { rel_path } = only(&report) else { panic!("{report:?}") };
    assert_eq!(rel_path, "nodes/setting/the-ember-coast.md");
    let landed = paths::from_rel_string(project.root(), rel_path);
    assert_eq!(fs::read_to_string(&landed).unwrap(), incoming.text);
    // And it is a real node afterwards, not just a file: the navigator reads the
    // index, so a create that did not index is a create the user cannot see.
    assert!(project.get_node(theirs.id).unwrap().notes_raw.contains("only nadia has"));
    assert!(project.list_nodes().unwrap().iter().any(|n| n.id == theirs.id));
}

#[test]
fn a_new_node_whose_filename_is_already_taken_parks_rather_than_overwrites() {
    // Two machines independently creating "Kael Vantris" produce two node ids
    // and one filename. The id is what identifies a node, so these are genuinely
    // two different characters — and the local one must not evaporate because a
    // peer got to the name too.
    let (_dir, mut project, node, path) = world();
    let ours = fs::read_to_string(&path).unwrap();

    let mut theirs = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    theirs.notes_raw = "a different kael entirely".into();
    assert_ne!(theirs.id, node.id);
    let incoming = Incoming {
        node_id: theirs.id,
        slug: theirs.slug.clone(),
        text: markdown::to_markdown(&theirs).unwrap(),
    };

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&incoming)).unwrap();

    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), ours, "our Kael was overwritten by theirs");
    assert_eq!(fs::read_to_string(&siblings(&path)[0]).unwrap(), incoming.text);
}

#[test]
fn a_node_deleted_on_one_side_is_left_alone_on_both() {
    // M3 has no tombstones, so the two available guesses are "resurrect it" and
    // "delete ours". The second, driven by an absence, turns a half-mounted
    // share into a world-wide erase; the first quietly undoes a deletion. Neither
    // is guessed, and the node stays where it is until deletions get a design.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let agreed = base(&project, NADIA, node.id);

    // Their side deleted it, so it is simply not in the batch — nothing arrives.
    // Ours deleted it, and their copy arrives anyway:
    fs::remove_file(&path).unwrap();
    let report =
        project.apply_from_peer(NADIA, &[from_peer(&node, "still here on nadia's")]).unwrap();

    assert_eq!(only(&report), &Applied::Deleted);
    assert!(!path.exists(), "a deletion was undone by a sync");
    assert!(siblings(&path).is_empty(), "a deletion raised a conflict card");
    assert_eq!(base(&project, NADIA, node.id), agreed, "a deletion moved the base");
    assert!(!report.changed_the_folder());
}

/* ── payloads that must be refused ────────────────────────────────────────── */

#[test]
fn bytes_that_are_not_the_node_they_claim_to_be_are_refused() {
    // The trust boundary. A peer that can announce one node and send another's
    // bytes can write any file in the folder, because the destination path comes
    // from the announcement and the contents come from the payload.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let ours = fs::read_to_string(&path).unwrap();

    let impostor = Node::new(NodeKind::Setting, "Somewhere Else").unwrap();
    let report = project
        .apply_from_peer(
            NADIA,
            &[Incoming {
                node_id: node.id,
                slug: node.slug.clone(),
                text: markdown::to_markdown(&impostor).unwrap(),
            }],
        )
        .unwrap();

    assert_eq!(
        only(&report),
        &Applied::Refused(Refused::WrongNode { contained: impostor.id }),
        "{report:?}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), ours);
    assert!(siblings(&path).is_empty(), "a refusal must not park anything either");
    assert_eq!(report.refusals().len(), 1);
}

#[test]
fn a_filename_that_is_not_a_slug_never_reaches_a_path() {
    // The only defence against a peer naming a node file `../../../../etc/cron.d`.
    // It has to sit on the near side of the join, so it is asserted at the API
    // boundary rather than trusted to `from_rel_string`.
    let (dir, mut project, _node, _path) = world();
    let theirs = Node::new(NodeKind::Setting, "Anywhere").unwrap();
    let text = markdown::to_markdown(&theirs).unwrap();

    for slug in ["../../escape", "/etc/passwd", "..", "has spaces", "", "Upper"] {
        let report = project
            .apply_from_peer(
                NADIA,
                &[Incoming { node_id: theirs.id, slug: slug.into(), text: text.clone() }],
            )
            .unwrap();
        assert_eq!(
            only(&report),
            &Applied::Refused(Refused::UnusableSlug { slug: slug.into() }),
            "{slug:?} was accepted"
        );
    }

    // Nothing was created anywhere under the project, and in particular nothing
    // above it.
    assert!(!dir.path().join("escape").exists());
    assert!(!project.root().join("nodes/setting").join("..").join("escape").exists());
}

#[test]
fn a_half_transferred_payload_is_refused_and_writes_nothing() {
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let ours = fs::read_to_string(&path).unwrap();

    let truncated =
        Incoming { node_id: node.id, slug: node.slug.clone(), text: "---\nid: 01JW".into() };
    let report = project.apply_from_peer(NADIA, &[truncated]).unwrap();

    assert!(matches!(only(&report), Applied::Refused(Refused::Unreadable { .. })), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), ours);
    assert!(siblings(&path).is_empty());
    assert!(!report.changed_the_folder());
}

#[test]
fn one_bad_payload_does_not_stop_the_good_ones_in_the_same_batch() {
    // A corrupt byte on one machine must not become a sync that never completes
    // for anybody. Refusals are per node, deliberately, and are not errors.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);

    let good = from_peer(&node, "this one is fine");
    let bad = Incoming { node_id: node.id, slug: node.slug.clone(), text: "not a node".into() };

    let report = project.apply_from_peer(NADIA, &[bad, good.clone()]).unwrap();

    assert_eq!(report.outcomes.len(), 2);
    assert!(matches!(report.outcomes[0].1, Applied::Refused(_)));
    assert!(matches!(report.outcomes[1].1, Applied::FastForwarded { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), good.text);
}

/* ── the batch ────────────────────────────────────────────────────────────── */

#[test]
fn a_batch_that_fails_partway_records_no_bases_at_all() {
    // Half a base set is worse than none, and the asymmetry is why. A base that
    // is *behind* costs a re-compare: the node that did land now reads
    // local == remote, which is `Converged`, and it catches up next round with
    // nothing written. A base that ran *ahead* of the folder would claim
    // agreement on bytes this machine does not have, and the next round would
    // fast-forward on that claim.
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let first = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let second = project.create_node(NodeKind::Character, "Vashk Orrin", None).unwrap();
    agree(&project, NADIA, first.id);
    agree(&project, NADIA, second.id);
    let agreed = base(&project, NADIA, first.id);

    let first_path = node_path(&project, first.id);
    let second_path = node_path(&project, second.id);

    // Make the second node unreadable in a way no amount of care in this crate
    // can recover from: something that is not a file is sitting where the file
    // should be. A wedged mount or a permission change would do as well.
    fs::remove_file(&second_path).unwrap();
    fs::create_dir(&second_path).unwrap();

    let batch = [from_peer(&first, "landed"), from_peer(&second, "never gets there")];
    let err = project.apply_from_peer(NADIA, &batch).unwrap_err();

    assert!(matches!(err, wobu_store::Error::Io { .. }), "{err}");
    assert_eq!(
        base(&project, NADIA, first.id),
        agreed,
        "a base was moved for a batch that did not finish"
    );

    // The file that did land is intact, and re-running the batch once the
    // obstruction is gone settles everything with nothing lost.
    assert_eq!(fs::read_to_string(&first_path).unwrap(), batch[0].text);
    fs::remove_dir(&second_path).unwrap();
    fs::write(&second_path, &batch[1].text).unwrap();
    let again = project.apply_from_peer(NADIA, &batch).unwrap();
    assert_eq!(again.outcomes[0].1, Applied::Converged);
    assert_eq!(again.outcomes[1].1, Applied::Converged);
}

#[test]
fn a_batch_settles_every_kind_of_outcome_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let ahead = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let behind = project.create_node(NodeKind::Character, "Vashk Orrin", None).unwrap();
    let clash = project.create_node(NodeKind::Character, "Mira Solenne", None).unwrap();
    for node in [&ahead, &behind, &clash] {
        agree(&project, NADIA, node.id);
    }

    // They edited `behind`; we edited `clash`; they edited `clash` too.
    we_edit(&mut project, clash.id, "ours");
    let ahead_unchanged = Incoming {
        node_id: ahead.id,
        slug: ahead.slug.clone(),
        text: fs::read_to_string(node_path(&project, ahead.id)).unwrap(),
    };

    let report = project
        .apply_from_peer(
            NADIA,
            &[ahead_unchanged, from_peer(&behind, "theirs"), from_peer(&clash, "theirs")],
        )
        .unwrap();

    assert_eq!(report.outcomes[0].1, Applied::InStep);
    assert!(matches!(report.outcomes[1].1, Applied::FastForwarded { .. }));
    assert!(matches!(report.outcomes[2].1, Applied::Conflicted { .. }));
    assert!(report.to_send().is_empty());
    assert_eq!(report.parked().len(), 1);
    assert!(report.changed_the_folder());
}

#[test]
fn an_empty_batch_is_a_no_op_rather_than_an_error() {
    let (_dir, mut project, _node, _path) = world();
    let report = project.apply_from_peer(NADIA, &[]).unwrap();
    assert!(report.outcomes.is_empty());
    assert!(!report.changed_the_folder());
}

/* ── one peer says nothing about another ──────────────────────────────────── */

#[test]
fn agreeing_with_one_peer_does_not_let_a_second_one_fast_forward() {
    // The row is a fact about a conversation, not about the file. Two peers can
    // legitimately be at different points in the same node's history, and
    // collapsing them would fast-forward the stale peer over an edit it never
    // saw.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    let ours = fs::read_to_string(&path).unwrap();

    // Priya has never synced this node with us at all.
    let report = project.apply_from_peer(PRIYA, &[from_peer(&node, "priya's version")]).unwrap();

    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), ours);
    assert_eq!(base(&project, PRIYA, node.id), None);
    assert!(base(&project, NADIA, node.id).is_some(), "the other conversation was disturbed");
}

#[test]
fn forgetting_a_peer_puts_that_conversation_back_to_the_start() {
    let (_dir, project, node, _path) = world();
    agree(&project, NADIA, node.id);
    agree(&project, PRIYA, node.id);

    project.forget_peer(NADIA).unwrap();

    assert_eq!(base(&project, NADIA, node.id), None);
    assert!(base(&project, PRIYA, node.id).is_some(), "the other share was revoked too");
}

/* ── the manifest diff ────────────────────────────────────────────────────── */

#[test]
fn a_manifest_diff_picks_the_work_without_touching_anything() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let settled = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let theirs_ahead = project.create_node(NodeKind::Character, "Vashk Orrin", None).unwrap();
    let ours_ahead = project.create_node(NodeKind::Character, "Mira Solenne", None).unwrap();
    // The singleton nodes a project is created with are in the manifest too, so
    // the peer is brought fully up to date first and the test then moves exactly
    // three things.
    agree_all(&project, NADIA);
    we_edit(&mut project, ours_ahead.id, "ours");

    let before: Vec<PathBuf> = walk(project.root());
    let remote: Vec<(Id, String)> = project
        .manifest()
        .unwrap()
        .into_iter()
        .map(|(id, _ours)| {
            // They moved on for one node; the rest they still hold as agreed.
            if id == theirs_ahead.id {
                (id, wobu_store::atomic::hash_bytes(b"a version only nadia has"))
            } else {
                (id, base(&project, NADIA, id).unwrap())
            }
        })
        .collect();

    let plan = project.plan_against_peer(NADIA, &remote).unwrap();

    assert_eq!(plan.wanted, vec![theirs_ahead.id], "asked for the wrong bodies");
    assert_eq!(plan.send, vec![ours_ahead.id]);
    assert!(plan.settled.is_empty(), "a node in step needs no base moved");
    assert!(plan.skipped.is_empty());
    assert!(!plan.is_empty());

    // Read-only in both directions: not one byte of the folder, and not one row
    // of `sync_state`. A plan is a proposal.
    assert_eq!(walk(project.root()), before, "planning wrote to the folder");
    let unchanged = base(&project, NADIA, settled.id).unwrap();
    assert_eq!(
        unchanged,
        remote.iter().find(|(id, _)| *id == settled.id).unwrap().1,
        "planning moved a base"
    );
}

#[test]
fn two_peers_in_step_have_nothing_to_do() {
    let (_dir, project, _node, _path) = world();
    agree_all(&project, NADIA);
    let plan = project.plan_against_peer(NADIA, &project.manifest().unwrap()).unwrap();
    assert!(plan.is_empty(), "{plan:?}");
}

#[test]
fn a_first_ever_sync_of_identical_folders_is_all_base_moving_and_no_transfer() {
    // The common case on the very first exchange between two machines that got
    // their copy of the world the same way — a copied folder, a git clone. Every
    // node reads as concurrent because there are no bases, and every node is
    // identical, so `Converged` is what keeps that from being a wall of conflict
    // cards for a folder where nothing whatsoever is wrong.
    let (_dir, project, node, _path) = world();
    assert_eq!(base(&project, NADIA, node.id), None);

    let plan = project.plan_against_peer(NADIA, &project.manifest().unwrap()).unwrap();

    assert!(plan.wanted.is_empty(), "identical folders asked for bytes");
    assert!(plan.send.is_empty());
    assert_eq!(plan.settled.len(), project.manifest().unwrap().len(), "{plan:?}");
    assert!(plan.settled.iter().any(|(id, _)| *id == node.id));

    project.record_agreed(NADIA, &plan.settled).unwrap();
    assert!(project.plan_against_peer(NADIA, &project.manifest().unwrap()).unwrap().is_empty());
}

#[test]
fn the_decision_table_is_reachable_without_a_project_at_all() {
    // `decide` is public because #79 and #82 have to be able to reason about a
    // manifest before any bytes exist. Pinned here so it stays part of the API
    // rather than an implementation detail somebody makes private.
    assert_eq!(Decision::FastForward, wobu_store::apply::decide(Some("a"), Some("b"), Some("a")));
    assert_eq!(Decision::Conflict, wobu_store::apply::decide(Some("a"), Some("b"), None));
}

/* ── the send half ────────────────────────────────────────────────────────── */

#[test]
fn what_we_send_is_what_is_on_disk_and_what_the_manifest_promised() {
    // A payload re-rendered from the index would be a second definition of the
    // file's contents, and the two would eventually disagree — at which point
    // the manifest describes one of them and the bytes are the other, and the
    // receiving machine fast-forwards onto a version nobody has.
    let (_dir, mut project, node, path) = world();
    we_edit(&mut project, node.id, "the paragraph as it really is");

    let out = project.outgoing(node.id).unwrap().expect("a node we hold");
    assert_eq!(out.node_id, node.id);
    assert_eq!(out.slug, "kael-vantris");
    assert_eq!(out.text, fs::read_to_string(&path).unwrap());
    assert_eq!(out.hash, wobu_store::atomic::hash_bytes(out.text.as_bytes()));

    let manifest = project.manifest().unwrap();
    assert_eq!(manifest.iter().find(|(id, _)| *id == node.id).unwrap().1, out.hash);

    // And it is exactly what the other end applies.
    let incoming: Incoming = out.into();
    assert_eq!(incoming.node_id, node.id);
    assert_eq!(incoming.slug, "kael-vantris");
}

#[test]
fn a_node_that_is_gone_is_nothing_to_send_rather_than_an_error() {
    // A node deleted between a manifest and a request for its bytes is ordinary,
    // and failing the whole sync over it would make deleting a character during
    // a poll interval into a stuck share.
    let (_dir, project, node, path) = world();
    fs::remove_file(&path).unwrap();
    assert_eq!(project.outgoing(node.id).unwrap(), None);
    assert_eq!(project.outgoing(wobu_core::new_id()).unwrap(), None);
}

/// Every file under a root, sorted — for asserting that something wrote nothing.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .collect();
    out.sort();
    out
}
