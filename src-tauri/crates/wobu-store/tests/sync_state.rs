//! What the index remembers about a peer, and what it deliberately forgets.
//!
//! `sync_state` holds one fact per (peer, node): the content hash both sides
//! last agreed on. That single row is what turns two hashes into a three-way
//! compare, and therefore what tells "we both edited it" apart from "one of us
//! did" — the distinction M3 exists to get right, and the one `guarded_write`
//! cannot make, because with replicas both peers write to their own folder
//! successfully and neither compare-and-swap ever fails.
//!
//! Two properties are worth more than the round trip, and both are here because
//! neither would be noticed if it broke:
//!
//! - **Losing a base is survivable.** With no base every node reads as
//!   concurrent, so the damage is a re-compare and some conflict cards nobody
//!   needed. That is why a schema bump is allowed to drop the table outright,
//!   and it is the reason this table can live in a store that is otherwise
//!   entirely disposable.
//! - **Inventing a base is not.** A base attributed to the wrong peer, or
//!   surviving a peer being forgotten, is a claim about bytes a machine holds
//!   that nobody ever verified — and #80 will fast-forward on it, writing one
//!   side's file over the other's without raising a conflict at all. Every
//!   isolation test below is guarding that asymmetry.

use wobu_core::{Id, Node, NodeKind, new_id};
use wobu_store::Index;
use wobu_store::atomic::Stamp;

/// Two peers, spelled the way iroh renders an `EndpointId`: hex, opaque, and
/// nothing the index is entitled to parse.
const ALPHA: &str = "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9";
const BETA: &str = "f9e8d7c6b5a4b3a2918071605f4e3d2c1b0af9e8d7c6b5a4b3a2918071605f4e";

/// A full-length BLAKE3 hex string, as `Stamp` produces — not the truncated
/// `source_version` form. The distinction matters: a version ignores names and
/// summaries by design, so basing sync on one would let a rename vanish.
fn hash(bytes: &[u8]) -> String {
    Stamp::of_bytes(bytes, 0).hash
}

/// Put a node in the index, so the tests that care about the table surviving
/// operations on nodes have something real to operate on.
fn indexed(index: &Index, node: &Node) -> Id {
    let rel = format!("nodes/{}/{}.md", node.kind.dir(), node.slug);
    index.upsert_node(node, &rel, &Stamp::of_bytes(b"x", 1)).unwrap();
    node.id
}

/* ── the round trip ───────────────────────────────────────────────────────── */

#[test]
fn a_base_survives_being_written_and_read_back() {
    let index = Index::in_memory().unwrap();
    let kael = new_id();

    assert_eq!(
        index.base_hash(ALPHA, kael).unwrap(),
        None,
        "a peer we have never synced with has agreed nothing, and that is not an error"
    );

    let agreed = hash(b"# Kael Vantris\n");
    index.record_base(ALPHA, kael, &agreed).unwrap();
    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), Some(agreed));
}

#[test]
fn agreeing_again_moves_the_base_rather_than_keeping_both() {
    // The primary key is what enforces this. A base is the *last* thing agreed,
    // not a history: two rows for one (peer, node) would leave the three-way
    // compare picking between them, and either answer is a guess.
    let index = Index::in_memory().unwrap();
    let kael = new_id();

    index.record_base(ALPHA, kael, &hash(b"first")).unwrap();
    let second = hash(b"second");
    index.record_base(ALPHA, kael, &second).unwrap();

    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), Some(second.clone()));
    let bases = index.bases_for_peer(ALPHA).unwrap();
    assert_eq!(bases.len(), 1, "an upsert, not an append");
    assert_eq!(bases.get(&kael), Some(&second));
}

#[test]
fn one_peers_agreement_says_nothing_about_another_peers() {
    // The row is a fact about a conversation, not about the file. Two peers can
    // legitimately be at different points in the same node's history — one
    // synced this morning, one last week — and collapsing them would fast
    // forward the stale peer over an edit it never saw.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    let (morning, week_ago) = (hash(b"current"), hash(b"older"));

    index.record_base(ALPHA, kael, &morning).unwrap();
    index.record_base(BETA, kael, &week_ago).unwrap();

    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), Some(morning));
    assert_eq!(index.base_hash(BETA, kael).unwrap(), Some(week_ago));
}

#[test]
fn a_base_is_scoped_to_its_node() {
    let index = Index::in_memory().unwrap();
    let (kael, vashk) = (new_id(), new_id());
    index.record_base(ALPHA, kael, &hash(b"kael")).unwrap();

    assert_eq!(index.base_hash(ALPHA, vashk).unwrap(), None);
}

/* ── the bulk paths ───────────────────────────────────────────────────────── */

#[test]
fn a_whole_peers_bases_come_back_in_one_read() {
    // What #79's manifest diff asks. It compares every node at once, and asking
    // per node would turn one indexed scan over a two-page table into a few
    // hundred round trips.
    let index = Index::in_memory().unwrap();
    let ours: Vec<(Id, String)> =
        (0..5).map(|n| (new_id(), hash(format!("node {n}").as_bytes()))).collect();
    index.record_bases(ALPHA, &ours).unwrap();
    index.record_bases(BETA, &[(ours[0].0, hash(b"beta is behind"))]).unwrap();

    let bases = index.bases_for_peer(ALPHA).unwrap();
    assert_eq!(bases.len(), 5, "one peer's manifest, not the whole table");
    for (id, expected) in &ours {
        assert_eq!(bases.get(id), Some(expected));
    }

    assert_eq!(index.bases_for_peer(BETA).unwrap().len(), 1);
    assert!(
        index.bases_for_peer("a peer we have never met").unwrap().is_empty(),
        "an unknown peer is an empty manifest, not a failure"
    );
}

#[test]
fn a_batch_lands_whole() {
    // #80 settles a work list and moves every base in it at the same moment.
    // Half a batch is not unsafe — the un-moved rows simply re-compare — but
    // "the bases as of the end of that sync" is a sentence somebody debugging a
    // spurious conflict can reason about, and "some prefix of them" is not.
    let index = Index::in_memory().unwrap();
    let batch: Vec<(Id, String)> =
        (0..200).map(|n| (new_id(), hash(format!("{n}").as_bytes()))).collect();
    index.record_bases(ALPHA, &batch).unwrap();

    let bases = index.bases_for_peer(ALPHA).unwrap();
    assert_eq!(bases.len(), 200);
    assert!(batch.iter().all(|(id, h)| bases.get(id) == Some(h)));
}

#[test]
fn an_empty_batch_is_a_no_op_rather_than_an_error() {
    // A sync where nothing needed settling is the common case once two peers
    // are in step, and it must not be a special case for the caller.
    let index = Index::in_memory().unwrap();
    let kael = new_id();
    index.record_base(ALPHA, kael, &hash(b"agreed")).unwrap();

    index.record_bases(ALPHA, &[]).unwrap();
    assert_eq!(index.bases_for_peer(ALPHA).unwrap().len(), 1);
}

/* ── forgetting ───────────────────────────────────────────────────────────── */

#[test]
fn forgetting_a_peer_leaves_every_other_peer_untouched() {
    // Revoking one share must not re-compare the world with everybody else. The
    // asymmetry is the point: forgetting too much costs a comparison, and
    // forgetting too little means trusting a shared history one side no longer
    // has any reason to hold.
    let index = Index::in_memory().unwrap();
    let (kael, vashk) = (new_id(), new_id());
    for node in [kael, vashk] {
        index.record_base(ALPHA, node, &hash(b"agreed")).unwrap();
        index.record_base(BETA, node, &hash(b"agreed")).unwrap();
    }

    index.forget_peer(ALPHA).unwrap();

    assert!(index.bases_for_peer(ALPHA).unwrap().is_empty());
    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), None);
    assert_eq!(index.bases_for_peer(BETA).unwrap().len(), 2, "the other share is unaffected");
}

#[test]
fn forgetting_a_peer_we_never_knew_is_quiet() {
    let index = Index::in_memory().unwrap();
    index.forget_peer(ALPHA).unwrap();
    assert!(index.bases_for_peer(ALPHA).unwrap().is_empty());
}

/* ── what must not disturb a base ─────────────────────────────────────────── */

#[test]
fn rebuilding_the_index_from_the_folder_keeps_the_bases() {
    // `clear` means "re-read the Markdown", and re-reading the Markdown says
    // nothing about what a peer held. The ids and hashes a base refers to come
    // out of a rescan unchanged, so a base that was true before it is still
    // true after — and a rebuild triggered by an impatient user must not
    // silently reset agreement with everyone and produce a wall of conflicts.
    let index = Index::in_memory().unwrap();
    let kael = indexed(&index, &Node::new(NodeKind::Character, "Kael Vantris").unwrap());
    let agreed = hash(b"# Kael Vantris\n");
    index.record_base(ALPHA, kael, &agreed).unwrap();

    index.clear().unwrap();

    assert!(index.is_empty().unwrap(), "the derived tables did go");
    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), Some(agreed));
}

#[test]
fn a_node_going_missing_does_not_reset_what_a_peer_held() {
    // `remove_node` runs during reconcile for files that have merely gone
    // missing — a share half-mounted, a sync client mid-write. A folder
    // blinking must not be able to erase agreement. Ids are ULIDs and never
    // reused, so the row left behind can only ever be read back for the node it
    // was written for.
    let index = Index::in_memory().unwrap();
    let kael = indexed(&index, &Node::new(NodeKind::Character, "Kael Vantris").unwrap());
    let agreed = hash(b"# Kael Vantris\n");
    index.record_base(ALPHA, kael, &agreed).unwrap();

    index.remove_node(kael).unwrap();

    assert_eq!(index.base_hash(ALPHA, kael).unwrap(), Some(agreed));
}

/* ── the schema bump ──────────────────────────────────────────────────────── */

#[test]
fn a_version_bump_drops_the_bases_with_everything_else() {
    // The reason this table is allowed to live in a disposable index. A schema
    // change throws it away with the derived tables and no migration runs,
    // which is only defensible because the cost is bounded: with no base every
    // node reads as concurrent, so the next sync re-compares and re-establishes
    // them. Nothing is lost that the peers cannot tell each other again.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.sqlite");
    let kael = new_id();

    {
        let index = Index::open_at(&path).unwrap();
        index.record_base(ALPHA, kael, &hash(b"agreed")).unwrap();
        assert!(index.base_hash(ALPHA, kael).unwrap().is_some());
    }

    // Pretend the file was written by a build with a different schema. Done
    // through a second connection rather than a private field, so the test
    // exercises the same path a real upgrade takes: open, notice, discard.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE meta SET value = '999' WHERE key = 'index_version'", []).unwrap();
    }

    let reopened = Index::open_at(&path).unwrap();
    assert_eq!(
        reopened.base_hash(ALPHA, kael).unwrap(),
        None,
        "a stale schema is discarded, bases included"
    );
    assert!(reopened.bases_for_peer(ALPHA).unwrap().is_empty());

    // And the table is usable again immediately — dropped and recreated, not
    // dropped and left missing until something else happens to run.
    let fresh = hash(b"re-established");
    reopened.record_base(ALPHA, kael, &fresh).unwrap();
    assert_eq!(reopened.base_hash(ALPHA, kael).unwrap(), Some(fresh));
}

#[test]
fn bases_survive_a_close_and_reopen() {
    // The whole point of the table being on disk. An agreement reached in one
    // session has to be there in the next, or every launch re-compares the
    // world with every peer.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("i.sqlite");
    let kael = new_id();
    let agreed = hash(b"# Kael Vantris\n");

    {
        let index = Index::open_at(&path).unwrap();
        index.record_base(ALPHA, kael, &agreed).unwrap();
    }

    let reopened = Index::open_at(&path).unwrap();
    assert_eq!(reopened.base_hash(ALPHA, kael).unwrap(), Some(agreed));
}
