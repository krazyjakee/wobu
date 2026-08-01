//! A conflict the user has already answered, and the one thing that must not be
//! answered for them.
//!
//! `three_way.rs` covers what happens when two replicas disagree. This file
//! covers what happens *afterwards*: the user pressed "keep mine", the sibling
//! went away, and the next sync round has to not park it again (#89).
//!
//! The mechanism is one row in `sync_rejected` — `(peer_id, node_id,
//! rejected_hash)` — and the entire risk in the feature is in that third column.
//! Without it the table would read "this peer's version of this node is refused",
//! which is a sentence about a *node* rather than about some bytes, and the
//! moment the peer writes a better paragraph it would vanish on this machine:
//! no card, no sibling, no trace. That is a lost edit, and it is a much worse
//! outcome than the redundant card this feature exists to remove. So the trap
//! gets the loudest tests here, at both layers — the table, and a real folder.
//!
//! The other property worth as much: **a refusal can only ever suppress.** It
//! turns "park a sibling" into "do nothing" and can reach no other arm of the
//! compare. If a row could reach a fast-forward, a mistaken or forged one would
//! be a database row authorising an overwrite of somebody's file.
//!
//! Peers are simulated exactly as in `three_way.rs`: another machine appears the
//! only way another machine ever can, as `Incoming` bytes with a node id on them.

use std::fs;
use std::path::{Path, PathBuf};

use wobu_core::{Id, Node, NodeKind, new_id};
use wobu_store::apply::{Applied, Incoming};
use wobu_store::atomic::{Stamp, hash_bytes};
use wobu_store::{Index, Keep, Project, markdown, paths};

/// Two peers, spelled the way iroh renders an `EndpointId`. The same pair
/// `three_way.rs` uses, so a reader following one file into the other is
/// following the same two machines.
const NADIA: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
const PRIYA: &str = "f9e8d7c6b5a4b3a2918071605f4e3d2c1b0af9e8d7c6b5a4b3a2918071605f4e";

/// A peer nothing in these tests ever agrees anything with, so the index has no
/// way to know it exists. Used for the honest-shrug case at the bottom.
const STRANGER: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/* ── the folder half ──────────────────────────────────────────────────────── */

fn world() -> (tempfile::TempDir, Project, Node, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Ashfall").unwrap();
    let node = project.create_node(NodeKind::Character, "Kael Vantris", None).unwrap();
    let rel = project.index().rel_path_of(node.id).unwrap().expect("indexed");
    let path = paths::from_rel_string(project.root(), &rel);
    (dir, project, node, path)
}

/// The bytes a peer sends: their copy of the node, rendered as their own
/// `save_node` would have rendered it.
fn from_peer(node: &Node, notes: &str) -> Incoming {
    let mut theirs = node.clone();
    theirs.notes_raw = notes.into();
    theirs.updated_at += chrono::Duration::seconds(30);
    Incoming {
        node_id: theirs.id,
        slug: theirs.slug.clone(),
        text: markdown::to_markdown(&theirs).unwrap(),
    }
}

/// What we last agreed with a peer about a node. Also what makes the peer
/// *known* to the index, which is what lets a sibling's alias be resolved back
/// to an id when the user answers the card.
fn agree(project: &Project, peer: &str, id: Id) {
    let hash = project.outgoing(id).unwrap().expect("on disk").hash;
    project.record_agreed(peer, &[(id, hash)]).unwrap();
}

fn we_edit(project: &mut Project, id: Id, notes: &str) {
    let mut ours = project.get_node(id).unwrap();
    ours.notes_raw = notes.into();
    project.save_node(ours).unwrap();
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

fn only(report: &wobu_store::ApplyReport) -> &Applied {
    assert_eq!(report.outcomes.len(), 1, "{report:?}");
    &report.outcomes[0].1
}

/// Park a peer's version, then answer the card with "keep mine" — the sequence
/// the whole issue is about. Returns the hash of the version that was refused.
fn refuse(project: &mut Project, peer: &str, incoming: &Incoming) -> String {
    let report = project.apply_from_peer(peer, std::slice::from_ref(incoming)).unwrap();
    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");

    let card = project.conflicts().unwrap().remove(0);
    let outcome = project.resolve_conflict(&card.rel_path, Keep::Current, &card.current_hash);
    assert_eq!(outcome.unwrap(), wobu_store::Resolved::Done);
    hash_bytes(incoming.text.as_bytes())
}

/* ── the bug ──────────────────────────────────────────────────────────────── */

#[test]
fn a_version_the_user_rejected_does_not_park_again() {
    // #89 in one test. The conflict deliberately did not move the base — moving
    // it would claim agreement on bytes nobody agreed to — so the same
    // disagreement is rediscovered on every round, and before this fix the card
    // came back for ever. A card that will not stay dismissed teaches people to
    // ignore conflict cards, which is the failure that costs text later.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "nadia's second act");
    refuse(&mut project, NADIA, &theirs);
    assert!(siblings(&path).is_empty(), "resolving should have removed the sibling");
    let ours = fs::read_to_string(&path).unwrap();

    // The same bytes, the next round, and the round after that.
    for round in 0..3 {
        let again = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
        assert_eq!(only(&again), &Applied::AlreadyRefused, "round {round}: {again:?}");
        assert!(siblings(&path).is_empty(), "round {round}: the card came back");
        assert!(!again.changed_the_folder(), "round {round}");
        assert!(again.parked().is_empty(), "round {round}");
    }

    // And nothing was written. "Keep mine" means the file on disk is untouched,
    // including by the bookkeeping that makes the card stay gone.
    assert_eq!(fs::read_to_string(&path).unwrap(), ours);
    assert!(project.get_node(node.id).unwrap().notes_raw.contains("ours"));
}

#[test]
fn refusing_a_version_never_moves_the_base() {
    // The line the fix is not allowed to cross. The tempting shortcut for #89
    // was always "move the base to the remote hash and the re-compare stops",
    // and it stops it by claiming the two machines agreed on bytes the user
    // just declined — after which a later fast-forward writes on that claim.
    // The refusal is recorded somewhere else precisely so this stays true.
    let (_dir, mut project, node, _path) = world();
    agree(&project, NADIA, node.id);
    let agreed = project.index().base_hash(NADIA, node.id).unwrap();
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "nadia's second act");
    refuse(&mut project, NADIA, &theirs);
    assert_eq!(project.index().base_hash(NADIA, node.id).unwrap(), agreed, "the base moved");

    project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(
        project.index().base_hash(NADIA, node.id).unwrap(),
        agreed,
        "suppressing the conflict quietly recorded an agreement instead",
    );
}

/* ── the trap ─────────────────────────────────────────────────────────────── */

#[test]
fn refusing_one_version_does_not_suppress_a_later_different_one() {
    // **The most important test in this file.** The user refused Tuesday's
    // paragraph. On Wednesday the same peer writes a better one and sends it.
    // If the refusal were keyed on (peer, node) rather than on the bytes,
    // Wednesday's paragraph would be dropped on the floor here — no conflict, no
    // sibling, no card, nothing in the folder and nothing on screen. That is a
    // genuine lost edit, and it is strictly worse than the redundant card this
    // whole feature exists to remove.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let tuesday = from_peer(&node, "nadia's second act");
    refuse(&mut project, NADIA, &tuesday);

    let wednesday = from_peer(&node, "nadia's second act, rewritten overnight");
    assert_ne!(wednesday.text, tuesday.text, "the two versions have to actually differ");

    let report = project.apply_from_peer(NADIA, std::slice::from_ref(&wednesday)).unwrap();
    assert!(matches!(only(&report), Applied::Conflicted { .. }), "{report:?}");
    assert!(report.changed_the_folder());

    // Not just a card: the paragraph itself has to be on disk, because the
    // sibling is the only copy of it this machine will ever hold.
    let parked = siblings(&path);
    assert_eq!(parked.len(), 1, "{parked:?}");
    assert_eq!(fs::read_to_string(&parked[0]).unwrap(), wednesday.text);
    assert_eq!(project.conflicts().unwrap().len(), 1);

    // Tuesday's is still refused, so the fix has not simply been undone by the
    // arrival of a second version.
    let repeat = project.apply_from_peer(NADIA, std::slice::from_ref(&tuesday)).unwrap();
    assert_eq!(only(&repeat), &Applied::AlreadyRefused, "{repeat:?}");
}

#[test]
fn refusing_two_versions_in_turn_leaves_both_refused() {
    // The set grows rather than being replaced. A node can be argued about more
    // than once — that is what `a_genuinely_different_second_version_still_parks`
    // in `three_way.rs` makes possible — and a row that overwrote the last
    // refusal would make the first card reappear the moment the second was
    // answered, which is #89 again with an extra step.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let first = from_peer(&node, "their first thought");
    let second = from_peer(&node, "their second thought");
    refuse(&mut project, NADIA, &first);
    refuse(&mut project, NADIA, &second);

    for version in [&first, &second] {
        let report = project.apply_from_peer(NADIA, std::slice::from_ref(version)).unwrap();
        assert_eq!(only(&report), &Applied::AlreadyRefused, "{report:?}");
    }
    assert!(siblings(&path).is_empty());
}

#[test]
fn a_refusal_is_about_one_peer_and_not_about_the_bytes_in_general() {
    // Two people can independently reach the same paragraph — a third machine
    // delivered it to both, most likely — and "I do not want Nadia's edit" is
    // not "I do not want anybody's edit". Suppressing Priya's copy would hide a
    // decision the user has never been asked to make, and hide it *silently*,
    // which is the shape of every bug this crate is written to avoid.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    agree(&project, PRIYA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "the same paragraph, twice");
    refuse(&mut project, NADIA, &theirs);

    let from_priya = project.apply_from_peer(PRIYA, std::slice::from_ref(&theirs)).unwrap();
    assert!(matches!(only(&from_priya), Applied::Conflicted { .. }), "{from_priya:?}");
    assert_eq!(siblings(&path).len(), 1);

    // And Nadia's is still refused, so this did not pass by forgetting the row.
    let from_nadia = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(only(&from_nadia), &Applied::AlreadyRefused, "{from_nadia:?}");
}

/* ── the other button ─────────────────────────────────────────────────────── */

#[test]
fn taking_the_peers_version_records_nothing_and_needs_no_cleanup() {
    // The half of the resolution that deliberately does no bookkeeping at all.
    // `Keep::Parked` writes their bytes to the node file, so the next compare
    // sees local == remote and reads `Converged` — and a refusal is only ever
    // consulted for a `Conflict`, which requires the two sides to differ. There
    // is therefore nothing to clear: any row about this node simply stops
    // matching. This test is here because that argument is easy to assert and
    // easy to be wrong about.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "nadia's second act");
    project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    let card = project.conflicts().unwrap().remove(0);
    project.resolve_conflict(&card.rel_path, Keep::Parked, &card.current_hash).unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), theirs.text);
    assert!(
        project.index().rejections_for_peer(NADIA).unwrap().is_empty(),
        "accepting a version recorded a refusal of it",
    );

    // The loop closes the way it did before #89: converged, base caught up,
    // nothing parked and nothing asked again.
    let after = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(only(&after), &Applied::Converged);
    assert_eq!(
        project.index().base_hash(NADIA, node.id).unwrap().as_deref(),
        Some(hash_bytes(theirs.text.as_bytes()).as_str()),
    );
    assert!(siblings(&path).is_empty());
}

#[test]
fn refusing_a_version_and_later_accepting_the_same_bytes_still_lands_it() {
    // The row from an earlier refusal must not become a veto. The user says no
    // to Nadia's paragraph, Priya sends the same paragraph, and the user changes
    // their mind and takes it. The bytes land, and — the part worth asserting —
    // the stale refusal for Nadia cannot reach anything afterwards, because the
    // two machines now hold identical bytes and identical bytes are never a
    // conflict.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    agree(&project, PRIYA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "the paragraph in question");
    refuse(&mut project, NADIA, &theirs);

    project.apply_from_peer(PRIYA, std::slice::from_ref(&theirs)).unwrap();
    let card = project.conflicts().unwrap().remove(0);
    project.resolve_conflict(&card.rel_path, Keep::Parked, &card.current_hash).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs.text);

    let after = project.apply_from_peer(NADIA, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(only(&after), &Applied::Converged, "a stale refusal blocked a convergence");
}

/* ── the manifest ─────────────────────────────────────────────────────────── */

#[test]
fn a_refused_version_is_not_asked_for_again() {
    // The other consult, on the read-only side. A plan that keeps asking for
    // bytes whose only destination is a sibling `apply` will decline to write is
    // a transfer with no outcome, repeated on every round for ever — which over
    // a slow link is the visible half of this bug.
    let (_dir, mut project, node, _path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "nadia's second act");
    let their_hash = hash_bytes(theirs.text.as_bytes());

    let before = project.plan_against_peer(NADIA, &[(node.id, their_hash.clone())]).unwrap();
    assert!(before.wanted.contains(&node.id), "{before:?}");

    refuse(&mut project, NADIA, &theirs);

    let after = project.plan_against_peer(NADIA, &[(node.id, their_hash)]).unwrap();
    assert!(after.wanted.is_empty(), "{after:?}");
    // Not `skipped` either — that sentence means "one side deleted this", and a
    // refused node belongs to no list at all. (The project's other nodes are in
    // `send`, because a manifest of one node says the peer is missing the rest.)
    assert!(!after.skipped.contains(&node.id), "a refusal is not a deletion: {after:?}");
    assert!(after.settled.is_empty(), "a refusal was recorded as an agreement: {after:?}");

    // A different version from the same peer is still wanted, for the same
    // reason it still conflicts.
    let wednesday = hash_bytes(from_peer(&node, "rewritten overnight").text.as_bytes());
    let later = project.plan_against_peer(NADIA, &[(node.id, wednesday)]).unwrap();
    assert!(later.wanted.contains(&node.id), "a later version went unasked for: {later:?}");
}

/* ── naming the peer behind an alias ──────────────────────────────────────── */

#[test]
fn a_sibling_from_a_peer_the_index_has_never_heard_of_records_nothing() {
    // The honest shrug, pinned so it stays honest. A conflict sibling carries a
    // peer *alias* and nothing else — that is all its filename can carry, since
    // the name has to read the same on every machine — and there is no table
    // mapping an alias back to an id. So the alias is inverted by re-deriving it
    // over the peers the index already knows, and a peer it has never agreed or
    // refused anything with is not in that set.
    //
    // The result is the old behaviour for that peer: the card comes back. That
    // is the right way to be wrong. The alternative is attaching a person's
    // decision to whichever machine happened to look similar, and a refusal
    // recorded against the wrong peer withholds that peer's work.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "from someone we have never settled with");
    refuse(&mut project, STRANGER, &theirs);

    assert!(project.index().rejections_for_peer(STRANGER).unwrap().is_empty());
    assert!(
        project.index().rejections_for_peer(NADIA).unwrap().is_empty(),
        "a refusal was attributed to the wrong peer, which withholds their work",
    );

    let again = project.apply_from_peer(STRANGER, std::slice::from_ref(&theirs)).unwrap();
    assert!(matches!(only(&again), Applied::Conflicted { .. }), "{again:?}");
    assert_eq!(siblings(&path).len(), 1);
}

#[test]
fn a_caller_holding_a_real_peer_id_can_refuse_without_the_guesswork() {
    // `wobu-sync`, or a shell command wired to a card that carries an id, does
    // not have to go through a filename at all. Same row, no inversion.
    let (_dir, mut project, node, path) = world();
    agree(&project, NADIA, node.id);
    we_edit(&mut project, node.id, "ours");

    let theirs = from_peer(&node, "from someone we have never settled with");
    project.reject_from_peer(STRANGER, node.id, &hash_bytes(theirs.text.as_bytes())).unwrap();

    let report = project.apply_from_peer(STRANGER, std::slice::from_ref(&theirs)).unwrap();
    assert_eq!(only(&report), &Applied::AlreadyRefused, "{report:?}");
    assert!(siblings(&path).is_empty());
}

/* ── the table on its own ─────────────────────────────────────────────────── */

fn hash(bytes: &[u8]) -> String {
    Stamp::of_bytes(bytes, 0).hash
}

#[test]
fn a_refusal_survives_being_written_and_read_back() {
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    let refused = hash(b"their version");

    assert!(!index.is_rejected(NADIA, kael, &refused).unwrap());
    index.record_rejection(NADIA, kael, &refused).unwrap();
    assert!(index.is_rejected(NADIA, kael, &refused).unwrap());

    // Idempotent, because the button is. Every column is in the primary key, so
    // a second press has nothing to say that the first did not.
    index.record_rejection(NADIA, kael, &refused).unwrap();
    assert_eq!(index.rejections_for_peer(NADIA).unwrap()[&kael].len(), 1);
}

#[test]
fn the_table_is_keyed_on_the_bytes_and_not_on_the_node() {
    // The trap again, at the layer that has to make it true. Everything above
    // this line in the crate is only as safe as this row is specific.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    let tuesday = hash(b"their tuesday");
    let wednesday = hash(b"their wednesday");

    index.record_rejection(NADIA, kael, &tuesday).unwrap();
    assert!(index.is_rejected(NADIA, kael, &tuesday).unwrap());
    assert!(!index.is_rejected(NADIA, kael, &wednesday).unwrap(), "a later edit was swallowed");

    index.record_rejection(NADIA, kael, &wednesday).unwrap();
    let set = &index.rejections_for_peer(NADIA).unwrap()[&kael];
    assert_eq!(set.len(), 2, "the second refusal replaced the first instead of joining it");
    assert!(set.contains(&tuesday) && set.contains(&wednesday));
}

#[test]
fn refusals_do_not_leak_between_peers_or_between_nodes() {
    let index = Index::in_memory().unwrap();
    let (kael, sera) = (new_id(), new_id());
    let bytes = hash(b"the same paragraph");

    index.record_rejection(NADIA, kael, &bytes).unwrap();

    assert!(!index.is_rejected(PRIYA, kael, &bytes).unwrap(), "a refusal crossed to another peer");
    assert!(!index.is_rejected(NADIA, sera, &bytes).unwrap(), "a refusal crossed to another node");
    assert!(index.rejections_for_peer(PRIYA).unwrap().is_empty());
}

#[test]
fn forgetting_a_peer_forgets_what_was_refused_from_it_too() {
    // A refusal is a fact about a relationship: this person, having seen who it
    // came from, said no. A relationship that has ended cannot leave one behind
    // — if the id ever returned it would be a share re-granted, and the first
    // thing a stale row would do is quietly withhold a version of the world that
    // a machine the user has just re-admitted is holding. That is the one way a
    // row in this table costs text rather than patience.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    let bytes = hash(b"their version");

    index.record_base(NADIA, kael, &hash(b"agreed")).unwrap();
    index.record_rejection(NADIA, kael, &bytes).unwrap();
    index.record_rejection(PRIYA, kael, &bytes).unwrap();

    index.forget_peer(NADIA).unwrap();

    assert!(!index.is_rejected(NADIA, kael, &bytes).unwrap());
    assert!(index.base_hash(NADIA, kael).unwrap().is_none());
    assert!(index.is_rejected(PRIYA, kael, &bytes).unwrap(), "forgetting one peer hit another");
}

#[test]
fn a_rebuild_does_not_forget_what_a_person_decided() {
    // `clear` means "re-read the Markdown", and re-reading the Markdown says
    // nothing about which versions somebody looked at and refused — exactly as
    // it says nothing about what a peer holds, which is why `sync_state` is not
    // in that list either. A user asking for an index repair for unrelated
    // reasons must not have every conflict card they have ever dismissed come
    // back at them on the next sync.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    let bytes = hash(b"their version");
    index.record_rejection(NADIA, kael, &bytes).unwrap();

    index.clear().unwrap();

    assert!(index.is_rejected(NADIA, kael, &bytes).unwrap());
}

#[test]
fn a_node_file_going_missing_does_not_resurrect_a_dismissed_card() {
    // `remove_node` runs during reconcile for files that have merely gone away
    // — a share half-mounted, a sync client mid-write — so a folder blinking
    // must not be able to undo a decision. Bases are kept through it for the
    // mirror-image reason, and ULIDs are never reused, so a row left behind can
    // only ever be read back for the node it was written for.
    let index = Index::in_memory().unwrap();
    let node = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
    let rel = format!("nodes/{}/{}.md", node.kind.dir(), node.slug);
    index.upsert_node(&node, &rel, &Stamp::of_bytes(b"x", 1)).unwrap();
    let bytes = hash(b"their version");
    index.record_rejection(NADIA, node.id, &bytes).unwrap();

    index.remove_node(node.id).unwrap();

    assert!(index.is_rejected(NADIA, node.id, &bytes).unwrap());
}

#[test]
fn a_version_bump_drops_the_refusals_and_rebuilds_clean() {
    // The 7 → 8 bump, and the argument for it being free. Losing the table costs
    // one redundant conflict card per rejected node — a person dismissing a card
    // they had dismissed before — and no correctness at all, which is the bar a
    // table is allowed to live in this index under. Nothing migrates.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.sqlite");
    let kael = new_id();
    let bytes = hash(b"their version");

    {
        let index = Index::open_at(&path).unwrap();
        index.record_rejection(NADIA, kael, &bytes).unwrap();
        assert!(index.is_rejected(NADIA, kael, &bytes).unwrap());
    }

    // Pretend the file was written by a build with a different schema, through a
    // second connection, so the test takes the same path a real upgrade does:
    // open, notice, discard.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = '999' WHERE key = 'index_version'", []).unwrap();
    }

    let reopened = Index::open_at(&path).unwrap();
    assert!(!reopened.is_rejected(NADIA, kael, &bytes).unwrap(), "a stale schema survived");
    assert!(reopened.rejections_for_peer(NADIA).unwrap().is_empty());

    // Dropped *and recreated*, not dropped and left missing until something else
    // happens to run. A build that skipped this would fail on the first refusal
    // after an upgrade, with `no such table`, inside a button press.
    reopened.record_rejection(NADIA, kael, &bytes).unwrap();
    assert!(reopened.is_rejected(NADIA, kael, &bytes).unwrap());
}

#[test]
fn a_refusal_survives_a_close_and_reopen() {
    // The whole point of it being on disk. A decision made in one session has to
    // still hold in the next, or the card comes back every launch — which is
    // #89 with a longer period and no easier to live with.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.sqlite");
    let kael = new_id();
    let bytes = hash(b"their version");

    {
        Index::open_at(&path).unwrap().record_rejection(NADIA, kael, &bytes).unwrap();
    }

    assert!(Index::open_at(&path).unwrap().is_rejected(NADIA, kael, &bytes).unwrap());
}

#[test]
fn a_peer_becomes_known_by_agreeing_or_by_being_refused() {
    // `known_peers` is what lets a sibling's alias be turned back into an id.
    // Sourcing it from both tables is load-bearing: once a peer has had a
    // refusal recorded it must stay resolvable even if every base with it is
    // later replaced or dropped, or the *second* refusal for that peer would
    // land nowhere and its card would start coming back again.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    assert!(index.known_peers().unwrap().is_empty());

    index.record_base(NADIA, kael, &hash(b"agreed")).unwrap();
    index.record_rejection(PRIYA, kael, &hash(b"refused")).unwrap();

    let mut known = index.known_peers().unwrap();
    known.sort();
    assert_eq!(known, vec![NADIA.to_string(), PRIYA.to_string()]);
}
