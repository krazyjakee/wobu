//! Manifests over a real connection: two endpoints, one process, one loopback
//! interface.
//!
//! Everything here goes through real QUIC — real streams, real flow control,
//! real ordering — because the parts of [#79] most likely to be wrong are the
//! parts a unit test cannot see. Paging that loses the last entry of a page,
//! duplex halves that deadlock the moment a manifest outgrows a receive window,
//! and a cap that is enforced on one side and not the other are all invisible to
//! a test that calls `serde_json::to_vec` and reads it straight back.
//!
//! What a green run here does *not* cover is the same list `loopback.rs` gives:
//! NAT traversal, holepunching, relay selection, and a link slow enough for the
//! silence deadline to be interesting. `Reach::Loopback` has no relay and no
//! address lookup so that no test in this crate can quietly acquire a dependency
//! on n0's infrastructure.
//!
//! [#79]: https://github.com/krazyjakee/wobu/issues/79

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use wobu_core::{Id, new_id};
use wobu_sync::manifest::{self, Blob, Counts, MAX_ENTRIES, PAGE_ENTRIES};
use wobu_sync::{Config, Error, Projects, Session, Sessions, SyncEndpoint};

/* ── the rig ──────────────────────────────────────────────────────────────── */

struct Held(Id);

impl Projects for Held {
    fn holds(&self, project: &Id) -> bool {
        *project == self.0
    }
}

struct Sink(mpsc::UnboundedSender<Session>);

#[async_trait]
impl Sessions for Sink {
    async fn opened(&self, session: Session) {
        let _ = self.0.send(session);
    }
}

/// A connected pair of sessions for one project, and the two endpoints that must
/// stay alive for as long as they do.
///
/// Both sessions are returned because a manifest exchange is symmetric: it is
/// not a thing one side does to the other, and a helper that handed back only
/// the dialling half would make every test here write the accepting half by
/// hand.
async fn pair() -> (SyncEndpoint, SyncEndpoint, Session, Session) {
    let project = new_id();
    let (sessions, mut inbox) = mpsc::unbounded_channel();
    let config = || Config { open_timeout: Duration::from_millis(500), ..Config::loopback() };

    let accepting =
        SyncEndpoint::bind(config(), Arc::new(Held(project)), Arc::new(Sink(sessions))).await;
    let accepting = accepting.expect("a loopback endpoint binds without a network");

    let (spare, _) = mpsc::unbounded_channel();
    let dialling = SyncEndpoint::bind(config(), Arc::new(Held(project)), Arc::new(Sink(spare)))
        .await
        .expect("a loopback endpoint binds without a network");

    let outbound = dialling.connect(accepting.addr(), project).await.expect("both hold it");
    let inbound = inbox.recv().await.expect("the accepting side saw the session");
    (accepting, dialling, inbound, outbound)
}

/// Twelve seconds of silence is far more than a loopback round trip and far less
/// than a test run people will wait for. Every test but the timeout one should
/// finish long before it matters.
const IDLE: Duration = Duration::from_secs(12);

/// A node entry that is shaped like a real one: a genuine ULID and sixty-four
/// lowercase hex characters, because [`manifest::is_content_hash`] refuses
/// anything else and a test using `"hash-1"` would be testing the refusal path
/// while believing it tested the happy one.
fn node(seed: u128) -> (Id, String) {
    (Id::from(seed), format!("{seed:032x}{seed:032x}"))
}

fn nodes(count: usize) -> Vec<(Id, String)> {
    (0..count as u128).map(node).collect()
}

fn blob(seed: u128) -> Blob {
    let hash = format!("{seed:032x}{seed:032x}");
    Blob { rel_path: format!("assets/originals/{}/{hash}.png", &hash[..2]), hash }
}

/// Run both halves of an exchange at once.
///
/// Not a convenience. Both sides have to be in the exchange concurrently — each
/// opens the stream it writes and waits for the one it reads — so a test that
/// awaited one side and then the other would hang on the first await, which is
/// exactly the deadlock the duplex design exists to rule out.
async fn swap(
    a: &Session,
    a_nodes: &[(Id, String)],
    a_blobs: &[Blob],
    b: &Session,
    b_nodes: &[(Id, String)],
    b_blobs: &[Blob],
) -> (manifest::Exchange, manifest::Exchange) {
    let (from_b, from_a) = tokio::join!(
        manifest::exchange(a, a_nodes, a_blobs, IDLE),
        manifest::exchange(b, b_nodes, b_blobs, IDLE),
    );
    (from_b.expect("a's view of b"), from_a.expect("b's view of a"))
}

/* ── the round trip ───────────────────────────────────────────────────────── */

/// The whole feature in one assertion, in both directions at once.
///
/// The two manifests are deliberately different and deliberately overlapping:
/// if the exchange ever crossed its wires and handed each side its *own* list
/// back, a test where both sides held the same thing would pass.
#[tokio::test]
async fn a_manifest_arrives_as_the_peer_wrote_it_in_both_directions() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let mine = nodes(3);
    let theirs: Vec<(Id, String)> = (10..17u128).map(node).collect();
    let my_blobs = vec![blob(1)];
    let their_blobs = vec![blob(2), blob(3)];

    let (from_them, from_us) =
        swap(&inbound, &mine, &my_blobs, &outbound, &theirs, &their_blobs).await;

    assert_eq!(from_them.nodes, theirs);
    assert_eq!(from_them.blobs, their_blobs);
    assert_eq!(from_us.nodes, mine);
    assert_eq!(from_us.blobs, my_blobs);
    assert!(from_them.is_whole() && from_us.is_whole());
    assert_eq!(from_them.elided(), Counts::default());
    assert_eq!(from_them.held, Counts { nodes: 7, blobs: 2 });
    assert_eq!(from_them.sent, Counts { nodes: 3, blobs: 1 }, "sent is our half, not theirs");
    assert_eq!(from_them.refused, 0);
}

/// A hash is not decoration. It is the entire content of the exchange, and it is
/// compared with `==` at the far end by `wobu_store::apply::decide`, so a byte
/// changed anywhere in the encoding would turn every node into a conflict on
/// first contact — and a conflict card per node is how a user learns to dismiss
/// them.
#[tokio::test]
async fn every_byte_of_every_hash_survives_the_wire() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    // Both ends of the hex alphabet and both ends of the ULID range, because a
    // truncation or a sign error shows up at the edges and nowhere else.
    let awkward = vec![
        (Id::from(0u128), "0".repeat(64)),
        (Id::from(u128::MAX), "f".repeat(64)),
        (new_id(), format!("{}{}", "0123456789abcdef".repeat(3), "0123456789abcdef")),
    ];

    let (from_them, _) = swap(&inbound, &[], &[], &outbound, &awkward, &[]).await;

    assert_eq!(from_them.nodes, awkward);
}

/// Two peers with nothing to say to each other still have to complete the
/// exchange, and completing it has to be distinguishable from failing at it.
/// This is the first sync of a project that was just created on both machines,
/// and it is also every poll after two peers have converged — so it is the most
/// common exchange there is, not an edge case.
#[tokio::test]
async fn an_empty_manifest_is_a_complete_exchange_and_not_a_failure() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;

    let (from_them, from_us) = swap(&inbound, &[], &[], &outbound, &[], &[]).await;

    for view in [&from_them, &from_us] {
        assert!(view.nodes.is_empty());
        assert!(view.blobs.is_empty());
        assert_eq!(view.held, Counts::default());
        assert_eq!(view.sent, Counts::default());
        assert_eq!(view.refused, 0);
        assert!(view.is_whole(), "an empty manifest is a whole manifest");
    }
}

/// One side holds a project, the other holds nothing yet: the clone case. The
/// asymmetry is the point — the empty side must not swallow the full one's
/// manifest, and the full side must not read the empty one's silence as a
/// failure to answer.
#[tokio::test]
async fn one_empty_side_and_one_full_side_both_get_what_they_asked_for() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let theirs = nodes(400);

    let (from_them, from_us) = swap(&inbound, &[], &[], &outbound, &theirs, &[]).await;

    assert_eq!(from_them.nodes.len(), 400);
    assert!(from_them.is_whole());
    assert!(from_us.nodes.is_empty());
    assert!(from_us.is_whole());
}

/* ── paging ───────────────────────────────────────────────────────────────── */

/// The paging boundary, from both sides of it and on it.
///
/// A manifest is chunked, and an off-by-one in the chunking loses either the
/// last entry of every page or the first of the next — which, because an absence
/// means "never had it" and never "deleted", would not fail loudly. It would
/// produce a sync that silently never converges on some nodes, which is far
/// harder to notice than a crash. So the boundary is asserted exactly rather
/// than approximately, and `PAGE_ENTRIES` is imported rather than written out so
/// that changing it moves the test with it.
#[tokio::test]
async fn a_manifest_that_straddles_the_page_boundary_arrives_entire() {
    for count in [
        1,
        PAGE_ENTRIES - 1,
        PAGE_ENTRIES,
        PAGE_ENTRIES + 1,
        2 * PAGE_ENTRIES,
        2 * PAGE_ENTRIES + 1,
    ] {
        let (_accepting, _dialling, inbound, outbound) = pair().await;
        let theirs = nodes(count);

        let (from_them, _) = swap(&inbound, &[], &[], &outbound, &theirs, &[]).await;

        assert_eq!(from_them.nodes, theirs, "{count} nodes did not survive paging");
        assert_eq!(from_them.held.nodes, count);
        assert!(from_them.is_whole());
    }
}

/// Nodes and blobs page independently and both land whole, on a boundary chosen
/// so that neither section ends where the other does. Sections written into one
/// shared page counter would show up here and nowhere else.
#[tokio::test]
async fn nodes_and_blobs_page_independently_in_one_exchange() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let their_nodes = nodes(PAGE_ENTRIES + 1);
    let their_blobs: Vec<Blob> = (0..PAGE_ENTRIES as u128 + 5).map(blob).collect();

    let (from_them, _) = swap(&inbound, &[], &[], &outbound, &their_nodes, &their_blobs).await;

    assert_eq!(from_them.nodes, their_nodes);
    assert_eq!(from_them.blobs, their_blobs);
    assert_eq!(from_them.held, Counts { nodes: PAGE_ENTRIES + 1, blobs: PAGE_ENTRIES + 5 });
    assert!(from_them.is_whole());
}

/// A manifest far larger than any receive window, in both directions at once.
///
/// This is the test the duplex design exists for. If either side wrote its whole
/// manifest before reading any of the peer's, both would block on flow control
/// with neither draining the other, and this would hang rather than fail — so it
/// is written with a timeout around it, because a hung test is a test nobody
/// reads the output of.
#[tokio::test]
async fn two_large_manifests_cross_at_once_without_either_side_stalling() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let mine = nodes(9_000);
    let theirs: Vec<(Id, String)> = (100_000..109_000u128).map(node).collect();

    let both = tokio::time::timeout(
        Duration::from_secs(60),
        swap(&inbound, &mine, &[], &outbound, &theirs, &[]),
    )
    .await;

    let (from_them, from_us) = both.expect("the two halves deadlocked against each other");
    assert_eq!(from_them.nodes, theirs);
    assert_eq!(from_us.nodes, mine);
    assert!(from_them.is_whole() && from_us.is_whole());
}

/* ── the cap ──────────────────────────────────────────────────────────────── */

/// Going over the cap degrades. It does not fail, and it does not lie about
/// having succeeded.
///
/// Three things are asserted and all three matter. The exchange completes, so a
/// project that has grown past the cap can still sync — including the deletion
/// that would bring it back under, which an outright refusal would make
/// impossible. Only `MAX_ENTRIES` are retained, so the cap is a real bound on
/// memory and not a suggestion. And `is_whole` is false with `elided` saying by
/// how much, so a caller cannot mistake a partial picture for convergence.
///
/// What is *not* asserted, because it is one crate over, is the reason this is
/// safe at all: the entries that did not arrive are absences, an absence means
/// "never had it", and `wobu_store::apply::decide` turns that into an offer to
/// send bytes the peer already has. Nothing is written and nothing is removed.
#[tokio::test]
async fn a_manifest_past_the_cap_is_cut_down_rather_than_refused() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let over = MAX_ENTRIES + 137;
    let theirs = nodes(over);

    let from_them = tokio::time::timeout(
        Duration::from_secs(120),
        manifest::exchange(&inbound, &[], &[], IDLE),
    );
    let from_us = manifest::exchange(&outbound, &theirs, &[], IDLE);
    let (from_them, _) = tokio::join!(from_them, from_us);
    let from_them = from_them.expect("the exchange hung").expect("the cap must degrade, not fail");

    assert_eq!(from_them.nodes.len(), MAX_ENTRIES, "the cap did not bound what was retained");
    assert_eq!(from_them.held.nodes, over, "the peer must still say what it really holds");
    assert!(!from_them.is_whole());
    assert_eq!(from_them.elided(), Counts { nodes: 137, blobs: 0 });
    // The prefix, in order: a cut manifest is the first N entries of the peer's,
    // not an arbitrary N of them, so two runs against an unchanged project cut in
    // the same place.
    assert_eq!(from_them.nodes, theirs[..MAX_ENTRIES]);
}

/// The other half of the cap: a sender does not put more on the wire than it
/// will accept back. Enforcing it only on the receiving side would make every
/// oversized project pay for megabytes that were always going to be discarded.
#[tokio::test]
async fn a_sender_stops_at_the_cap_and_says_how_far_over_it_was() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let mine = nodes(MAX_ENTRIES + 40);

    let ours = tokio::time::timeout(Duration::from_secs(120), async {
        let (from_them, _) = tokio::join!(
            manifest::exchange(&inbound, &mine, &[], IDLE),
            manifest::exchange(&outbound, &[], &[], IDLE),
        );
        from_them
    })
    .await
    .expect("the exchange hung")
    .expect("the cap must degrade, not fail");

    assert_eq!(ours.sent.nodes, MAX_ENTRIES, "more than the cap went on the wire");
}

/* ── what a peer sends that we will not pass on ───────────────────────────── */

/// One unusable entry must not cost the other two hundred good ones.
///
/// The same rule `wobu_store::apply::Refused` states for payloads, applied to
/// the manifest: a peer with one corrupt index row, or one running a build that
/// writes something this one cannot read, must not be able to stop the whole
/// project syncing. The bad entries are dropped and counted, which reads
/// downstream as an absence — and an absence is never a deletion.
#[tokio::test]
async fn entries_this_build_will_not_pass_on_are_dropped_and_the_rest_still_land() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let good = node(7);
    let their_nodes = vec![
        (Id::from(1u128), "not a hash".to_string()),
        good.clone(),
        // Right alphabet, wrong length.
        (Id::from(2u128), "ab".repeat(20)),
        // Right length, wrong case: two spellings of one digest would read as two
        // different files at the far end.
        (Id::from(3u128), "A".repeat(64)),
    ];
    let ok = blob(9);
    let their_blobs = vec![
        Blob { rel_path: "../../.ssh/authorized_keys".into(), hash: "0".repeat(64) },
        Blob { rel_path: "nodes/character/kael-vantris.md".into(), hash: "0".repeat(64) },
        ok.clone(),
        Blob { rel_path: "assets/originals/ab/x.png".into(), hash: "nope".into() },
    ];

    let (from_them, _) = swap(&inbound, &[], &[], &outbound, &their_nodes, &their_blobs).await;

    assert_eq!(from_them.nodes, vec![good]);
    assert_eq!(from_them.blobs, vec![ok]);
    assert_eq!(from_them.refused, 6);
    // The peer counted four and four; three and three never made it. That is the
    // same shape as the cap, and the caller reads it the same way.
    assert_eq!(from_them.held, Counts { nodes: 4, blobs: 4 });
    assert!(!from_them.is_whole());
}

/// A node id in one manifest and not the other is reported as being in one
/// manifest and not the other, and that is *all* this crate says about it.
///
/// There is no deleted list, no tombstone and no flag, because there is nothing
/// on the wire that could distinguish "they removed it" from "they never had
/// it". If a future change adds a way for this crate to express a deletion, this
/// test is where it will first look wrong — and the crate documentation is where
/// the argument against it is written down.
#[tokio::test]
async fn a_node_missing_from_a_peers_manifest_is_missing_and_nothing_more() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let shared = node(1);
    let only_ours = node(2);
    let mine = vec![shared.clone(), only_ours.clone()];
    let theirs = vec![shared.clone()];

    let (from_them, _) = swap(&inbound, &mine, &[], &outbound, &theirs, &[]).await;

    assert_eq!(from_them.nodes, theirs);
    assert!(!from_them.nodes.iter().any(|(id, _)| *id == only_ours.0));
    // Whole, and it has to be: the peer told us everything it has. The node we
    // hold and it does not is a difference for `wobu_store::apply::decide` to
    // rule on with a base hash it keeps locally, and the exchange has no opinion.
    assert!(from_them.is_whole());
    assert_eq!(from_them.held, Counts { nodes: 1, blobs: 0 });
}

/* ── a peer that stops talking ────────────────────────────────────────────── */

/// A session where only one side ever enters the exchange must end, and end as
/// an error rather than as an empty manifest.
///
/// Both failure modes it rules out are real. Hanging holds a task and a
/// connection for the life of the process, which is what the opening exchange's
/// deadline exists to prevent one round trip earlier. Returning an empty
/// manifest would be worse: it is a peer *stating* it holds nothing, and this
/// project would then offer to send it every node it has, forever, to a machine
/// that never asked.
#[tokio::test]
async fn a_peer_that_never_enters_the_exchange_is_a_timeout_and_not_an_empty_manifest() {
    let (_accepting, _dialling, inbound, _outbound) = pair().await;

    let alone = manifest::exchange(&inbound, &nodes(2), &[], Duration::from_millis(300)).await;

    assert!(matches!(alone, Err(Error::ManifestTimedOut { .. })), "{alone:?}");
}

/// A connection that dies mid-exchange is an error, not a short manifest.
///
/// The distinction is the one above, restated for the case that actually
/// happens: laptops close, wifi drops, and a half-read manifest looks exactly
/// like a peer that has deleted everything after the last page that arrived.
#[tokio::test]
async fn a_connection_that_dies_mid_exchange_does_not_report_what_it_managed_to_read() {
    let (_accepting, dialling, inbound, outbound) = pair().await;
    let mine = nodes(4);

    let cut = tokio::time::timeout(Duration::from_secs(30), async {
        let ours = manifest::exchange(&inbound, &mine, &[], IDLE);
        let theirs = async {
            // Far enough into the exchange that pages have crossed, then gone.
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(outbound);
            dialling.shutdown().await.unwrap();
        };
        let (ours, ()) = tokio::join!(ours, theirs);
        ours
    })
    .await
    .expect("a severed exchange hung instead of failing");

    assert!(cut.is_err(), "a severed exchange reported a manifest: {cut:?}");
}

/* ── a peer writing the stream by hand ────────────────────────────────────── */

/// Write bytes down a manifest stream without going through [`manifest`], and
/// see what the other side makes of them.
///
/// The half of the protocol no honest peer exercises. Everything this crate
/// sends is well-formed by construction, so the refusals can only be reached by
/// a peer running a different build, a corrupt one, or one that means harm —
/// which is to say by writing the stream by hand, here.
async fn writes_by_hand(session: &Session, bytes: &[u8]) {
    let mut send = session.connection().open_uni().await.unwrap();
    // The peer may hang up the moment it refuses, which is the point of the
    // test; a write that lost the race is not a failure of it.
    let _ = send.write_all(bytes).await;
    let _ = send.finish();
}

/// The allocation bound, which is the only one that matters before the entries
/// have been counted.
///
/// A reader using `read_to_end`, or growing its buffer to fit whatever arrived,
/// hands a stranger the size of an allocation. The line ceiling is this build's
/// constant and there is deliberately no length prefix for a peer to disagree
/// with it — so the only way to find out a page is too big is to stop reading
/// once it is, which is what this asserts.
#[tokio::test]
async fn a_page_larger_than_this_build_will_read_is_refused_rather_than_accommodated() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    // No newline anywhere in it: an endless line is the shape that would grow a
    // buffer without bound.
    let endless = vec![b'x'; manifest::MAX_PAGE_BYTES + 4096];

    let (ours, ()) = tokio::join!(
        manifest::exchange(&inbound, &[], &[], IDLE),
        writes_by_hand(&outbound, &endless),
    );

    assert!(matches!(ours, Err(Error::ManifestMalformed)), "{ours:?}");
}

/// A stream that stops without saying it has stopped is an error, not a short
/// manifest — including when it carried perfectly good pages first.
///
/// This is the same refusal as the severed connection above, reached the other
/// way. Both exist because the alternative is treating a cut stream as a peer
/// that holds nothing, and a peer that holds nothing is a peer we would offer to
/// send an entire project to. `End` is the only thing that makes a manifest
/// complete, and it carries the counts precisely so that "finished" is something
/// stated rather than inferred from a socket.
#[tokio::test]
async fn a_stream_that_ends_without_saying_so_is_not_a_manifest() {
    for written in [
        // Nothing at all.
        &b""[..],
        // A page, then silence.
        &b"{\"page\":\"nodes\",\"entries\":[]}\n"[..],
        // Half a page: the newline never came.
        &b"{\"page\":\"nodes\",\"entr"[..],
    ] {
        let (_accepting, _dialling, inbound, outbound) = pair().await;

        let (ours, ()) =
            tokio::join!(manifest::exchange(&inbound, &[], &[], IDLE), writes_by_hand(&outbound, written));

        assert!(matches!(ours, Err(Error::ManifestMalformed)), "{written:?} gave {ours:?}");
    }
}

/// Bytes that are not `wobu/sync/1` are refused without being described.
///
/// The refusal carries nothing, for the reason [`Error::Malformed`] carries
/// nothing one round trip earlier: what failed to parse was written by a peer,
/// and an error that quoted it would be a log line a stranger composes.
#[tokio::test]
async fn bytes_that_are_not_a_manifest_are_refused_without_being_quoted() {
    let secret = "0123456789abcdef-not-really-a-secret-but-treat-it-as-one";
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let junk = format!("hello {secret}\n");

    let (ours, ()) = tokio::join!(
        manifest::exchange(&inbound, &[], &[], IDLE),
        writes_by_hand(&outbound, junk.as_bytes()),
    );

    let refused = ours.expect_err("junk parsed as a manifest");
    assert!(matches!(refused, Error::ManifestMalformed), "{refused:?}");
    let told = format!("{refused} {refused:?}");
    assert!(!told.contains(secret), "the refusal quoted what the peer wrote: {told}");
}

/// A page this build does not recognise is refused rather than skipped.
///
/// Skipping would be the friendlier-looking choice and it is the wrong one here:
/// a section this build cannot read is a section of the peer's project it cannot
/// see, and carrying on would report a manifest that is silently missing part of
/// itself — an absence, which is read as "never had it". The ALPN is the version
/// (see [`wobu_sync`]), so a peer speaking a later one never reaches this code at
/// all; anything that does get here is corrupt rather than newer.
#[tokio::test]
async fn a_page_kind_this_build_does_not_know_is_refused_rather_than_skipped() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;

    let (ours, ()) = tokio::join!(
        manifest::exchange(&inbound, &[], &[], IDLE),
        writes_by_hand(&outbound, b"{\"page\":\"tombstones\",\"entries\":[]}\n"),
    );

    assert!(matches!(ours, Err(Error::ManifestMalformed)), "{ours:?}");
}

/* ── the connection is still usable afterwards ────────────────────────────── */

/// The exchange leaves the connection alone.
///
/// #81 runs on this connection after the manifest has crossed, so an exchange
/// that closed anything, reset anything or left a stream half-read would break
/// the next thing rather than itself — and it would break it intermittently,
/// which is the worst way to find out. Two exchanges back to back is also how
/// #82 would poll a long-lived session.
#[tokio::test]
async fn a_connection_survives_an_exchange_and_will_do_another() {
    let (_accepting, _dialling, inbound, outbound) = pair().await;
    let first = nodes(PAGE_ENTRIES + 3);

    let (from_them, _) = swap(&inbound, &[], &[], &outbound, &first, &[]).await;
    assert_eq!(from_them.nodes, first);

    // A second exchange on the same session, with a different manifest, as if
    // the peer had saved something in between.
    let second = nodes(PAGE_ENTRIES + 9);
    let (again, _) = swap(&inbound, &[], &[], &outbound, &second, &[]).await;
    assert_eq!(again.nodes, second);

    // And an ordinary stream still works afterwards, which is the seam
    // `loopback.rs` pins for the opening exchange, re-checked one protocol later.
    let (mut send, _recv) = outbound.connection().open_bi().await.unwrap();
    send.write_all(b"blobs next").await.unwrap();
    send.finish().unwrap();
    let (_send, mut recv) = inbound.connection().accept_bi().await.unwrap();
    assert_eq!(recv.read_to_end(64).await.unwrap(), b"blobs next");
}
