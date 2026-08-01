//! One round: what two peers do to a project once they have agreed which
//! project it is.
//!
//! The shape is #80's, restated here because this is the file that has to obey
//! it: `manifest()` ⇄ theirs → `plan_against_peer` → ask for `plan.wanted` →
//! `apply_from_peer` → emit `world:changed` if the folder moved → push
//! `report.to_send()` → `record_agreed` **on acknowledgement only**.
//!
//! ## Symmetric, because a `Session` is
//!
//! `SyncEndpoint::connect` and `Sessions::opened` hand out the same [`Session`]
//! type, deliberately, so nothing downstream has to know which end of the
//! connection it is. This function is therefore the same on both sides and is
//! called once per session on each. It would be shorter written as a
//! client/server pair, and it would then have a bug that only appears when two
//! people press "sync" at the same moment.
//!
//! Both halves run at once under `try_join!`: this side asks the peer for what
//! it wants while answering what the peer asks. That is not a throughput
//! decision. A round that asked everything before answering anything would
//! deadlock against a peer doing the same, and since both sides run *this* code
//! it would deadlock against itself — the same trap `manifest::exchange`
//! documents for its own two directions, one layer up.
//!
//! ## Termination
//!
//! The serving half runs until the peer says [`Done`](super::bodies::done). Not
//! until *this* side is finished — that is the subtle version and it is wrong:
//! this side can be done asking while the peer still has bodies to fetch, and a
//! server that stopped when its own client did would leave the peer's last
//! request unanswered and its round timing out. So: a side stops serving when
//! told to, and says it is done when it is. Both loops end exactly once both
//! sides have stopped asking.
//!
//! ## Reconcile first
//!
//! #80 flagged this and it is the sharpest thing in the file. `apply` writes to
//! the path *the index* holds for a node. A rename done in Obsidian, or by
//! anybody else with the folder mounted, is not in the index until `reconcile`
//! has run — so a round that skipped it would fast-forward a peer's version onto
//! the *old* path, leaving the renamed file untouched beside a resurrected one.
//! Two files, one node, and the index pointing at the wrong one.
//!
//! It is one directory walk against an index that is almost always already
//! right, and it is the same call the folder watcher makes on every event. There
//! is no version of this round that is allowed to skip it.
//!
//! ## Small batches, and what the size is actually about
//!
//! [`BATCH`](super::bodies::BATCH) is sixteen, and it is not a memory bound —
//! `bodies` already caps a line and a stream. It is about `ensure_writable`,
//! which `apply_from_peer` checks **once per call**. A share that unmounts
//! mid-batch leaves the mountpoint behind as an empty directory, so writes
//! afterwards succeed onto the local disk underneath it, where nobody will ever
//! see them — and sync is the one writer that runs with nobody watching, so it
//! is the one most likely to do that a hundred times before anyone notices.
//! Sixteen nodes is a fraction of a second, so the window is a fraction of a
//! second wide.
//!
//! ## The base moves on acknowledgement and on nothing else
//!
//! A base is a claim that a specific machine holds specific bytes, and the next
//! round fast-forwards on that claim without asking anybody. This file therefore
//! calls `record_agreed` in exactly three places, and each is worth naming:
//!
//! - `plan.settled` — both sides already hold identical bytes. Nothing has to
//!   move for that to be true.
//! - after a push the peer acknowledged, **paired with the hashes this side
//!   sent** rather than any the peer echoed. See
//!   [`Answer::Agreed`](super::bodies).
//! - inside `apply_from_peer`, which `wobu-store` does for itself for the nodes
//!   it actually wrote.
//!
//! There is deliberately no fourth. Moving a base when a node goes *on the
//! wire* would turn a dropped connection into a base describing a file the peer
//! never received, and the peer's next edit would then read as a one-sided
//! change and overwrite ours.

use wobu_core::Id;
use wobu_store::{Applied, ApplyReport, Incoming, Outgoing, Plan};
use wobu_sync::{Blob, Session, manifest};

use crate::error::{Code, CommandResult, WobuError};
use crate::sync::bodies::{self, BATCH, Request};
use crate::sync::manager::{Replica, SyncManager};

/// What a round did. Everything here is a count rather than a list, because the
/// caller's only questions are "was that worth doing" (the backoff) and "should
/// the window refetch".
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// Nodes this side wrote or parked from the peer.
    pub applied: usize,
    /// Conflict siblings this round created. Not the same as `applied`: a
    /// conflict changes the folder without anybody's version being replaced.
    pub parked: usize,
    /// Nodes the peer acknowledged receiving from us.
    pub pushed: usize,
    /// Nodes this side handed over because the peer asked.
    pub served: usize,
    /// Whether the project folder moved, either half.
    pub changed: bool,
    /// Whether the peer's manifest crossed entire. `false` means a cap bit
    /// somewhere and this sync cannot have converged, however clean it looked.
    pub whole: bool,
}

impl Outcome {
    /// Whether this round was worth the dial.
    ///
    /// Drives the poller's backoff, and is the whole of #80's "back off on a
    /// permanent conflict": two peers each holding a version the other refuses
    /// will exchange manifests, decide there is nothing to do, write nothing,
    /// and be here again — reporting `false` every time, which walks the
    /// interval down to its cap instead of polling hot for ever.
    pub fn did_something(&self) -> bool {
        self.applied > 0 || self.pushed > 0 || self.served > 0
    }
}

/// Run one round over an open session.
pub async fn run(
    manager: &SyncManager,
    replica: &Replica,
    session: &Session,
) -> CommandResult<Outcome> {
    // The 64-hex `EndpointId`, which is a TLS identity rather than anything the
    // peer told us. Never the alias: that is twenty-eight bits and a display
    // name, and every base and every refusal in the index is keyed by this.
    let peer = session.peer().to_string();

    // See the module documentation. This is not optional and not an
    // optimisation.
    replica.with(|project| {
        project.reconcile()?;
        Ok(())
    })?;

    let nodes = replica.with(|project| Ok(project.manifest()?))?;
    let announce = blobs::announce();

    let exchange = manifest::exchange(session, &nodes, &announce, manifest::IDLE_TIMEOUT).await?;
    blobs::received(&exchange.blobs);

    let plan = replica.with(|project| Ok(project.plan_against_peer(&peer, &exchange.nodes)?))?;

    // Recorded before any bytes move, and that is safe rather than eager:
    // `settled` is the set both sides already hold identical bytes for, so the
    // claim a base makes is already true. Waiting for the transfers would mean a
    // round that failed halfway leaving agreements unrecorded that nothing has
    // to happen for.
    if !plan.settled.is_empty() {
        replica.with(|project| Ok(project.record_agreed(&peer, &plan.settled)?))?;
    }

    let connection = session.connection();
    let (ours, theirs) = tokio::try_join!(
        ask(manager, replica, &peer, connection, &plan),
        answer(manager, replica, &peer, connection),
    )?;

    let outcome = Outcome {
        applied: ours.applied + theirs.applied,
        parked: ours.parked + theirs.parked,
        pushed: ours.pushed,
        served: theirs.served,
        changed: ours.changed || theirs.changed,
        whole: exchange.is_whole(),
    };

    // Once per round rather than once per node. Two hundred nodes firing two
    // hundred refetches would be worse than not refetching at all, and
    // `wobu-store` cannot emit — it has no `AppHandle` and must not learn how.
    if outcome.changed {
        manager.wake().world_changed(replica.project());
    }
    Ok(outcome)
}

/// The counts one half of a round produced.
#[derive(Default)]
struct Half {
    applied: usize,
    parked: usize,
    pushed: usize,
    served: usize,
    changed: bool,
}

/// This side's half: fetch what the plan wants, then push what the peer is
/// behind on, then say we are finished.
async fn ask(
    manager: &SyncManager,
    replica: &Replica,
    peer: &str,
    connection: &iroh::endpoint::Connection,
    plan: &Plan,
) -> CommandResult<Half> {
    let mut half = Half::default();

    // Everything the peer is behind on: what the manifest said, plus whatever
    // applying their bodies turned up. Their bodies can reveal more — a node we
    // fetched and found ourselves ahead of is one they need — so the second list
    // is not redundant with the first.
    let mut to_send: Vec<Id> = plan.send.clone();

    for batch in plan.wanted.chunks(BATCH) {
        if manager.stopping() {
            return Err(stopped());
        }
        let bodies = bodies::want(connection, batch).await?;
        if bodies.is_empty() {
            continue;
        }
        let incoming: Vec<Incoming> = bodies.into_iter().map(Incoming::from).collect();
        let report = replica.with(|project| Ok(project.apply_from_peer(peer, &incoming)?))?;
        absorb(&mut half, &report);
        to_send.extend(report.to_send());
    }

    // Sorted and deduplicated, so a node the manifest and an apply both named is
    // sent once. A work list that depends on which of the two got there first is
    // one nobody can diff against the last run while debugging a sync.
    to_send.sort_unstable();
    to_send.dedup();

    for batch in to_send.chunks(BATCH) {
        if manager.stopping() {
            return Err(stopped());
        }
        let outgoing = replica.with(|project| {
            let mut out = Vec::with_capacity(batch.len());
            for id in batch {
                // `None` is ordinary: a node deleted, or whose file went away,
                // between the plan and now. There is simply nothing to send.
                if let Some(node) = project.outgoing(*id)? {
                    out.push(node);
                }
            }
            Ok(out)
        })?;
        if outgoing.is_empty() {
            continue;
        }

        let acknowledged = bodies::give(connection, &outgoing).await?;
        // Paired against what *we* sent. The peer named ids; the hashes are
        // ours. See the module documentation, and `bodies::Answer::Agreed`.
        let agreed: Vec<(Id, String)> = outgoing
            .iter()
            .filter(|node| acknowledged.contains(&node.node_id))
            .map(|node| (node.node_id, node.hash.clone()))
            .collect();
        half.pushed += agreed.len();
        if !agreed.is_empty() {
            replica.with(|project| Ok(project.record_agreed(peer, &agreed)?))?;
        }
    }

    bodies::done(connection).await?;
    Ok(half)
}

/// The peer's half: answer whatever it asks until it says it is finished.
async fn answer(
    manager: &SyncManager,
    replica: &Replica,
    peer: &str,
    connection: &iroh::endpoint::Connection,
) -> CommandResult<Half> {
    let mut half = Half::default();

    loop {
        let (mut send, request) = bodies::accept(connection).await?;
        match request {
            Request::Want(ids) => {
                let outgoing: Vec<Outgoing> = replica.with(|project| {
                    let mut out = Vec::with_capacity(ids.len());
                    for id in &ids {
                        if let Some(node) = project.outgoing(*id)? {
                            out.push(node);
                        }
                    }
                    Ok(out)
                })?;
                half.served += outgoing.len();
                bodies::bodies(&mut send, &outgoing).await?;
            }
            Request::Give(nodes) => {
                let incoming: Vec<Incoming> = nodes.into_iter().map(Incoming::from).collect();
                let report =
                    replica.with(|project| Ok(project.apply_from_peer(peer, &incoming)?))?;
                absorb(&mut half, &report);
                // Acknowledged *after* the write, which is the only ordering
                // that makes an acknowledgement mean anything: the peer moves a
                // base on the strength of this, and a base is what licenses a
                // later fast-forward.
                bodies::agreed(&mut send, &agreed(&report)).await?;
            }
            Request::Done => {
                bodies::finished(&mut send).await?;
                return Ok(half);
            }
        }

        if manager.stopping() {
            return Err(stopped());
        }
    }
}

/// Drive the production receiving half until a deliberately broken peer makes
/// it fail. #85 uses this wrapper so the cut-transfer test exercises the exact
/// `answer`/`apply_from_peer` path without exposing private counters.
#[cfg(test)]
pub(super) async fn answer_until_cut(
    manager: &SyncManager,
    replica: &Replica,
    peer: &str,
    connection: &iroh::endpoint::Connection,
) -> CommandResult<()> {
    answer(manager, replica, peer, connection).await.map(|_| ())
}

/// The round was told to stop.
///
/// Returned rather than broken out of, and that is the whole of it: `try_join!`
/// cancels its sibling on an error and on nothing else. A half that returned
/// `Ok` early would leave the other half waiting for a peer nobody is going to
/// answer, for the length of an idle timeout, at exactly the moment the app is
/// trying to quit — which is the shutdown hang wearing a different hat.
///
/// Nothing is left half-written by it. Every write inside a round is one
/// `guarded_write` plus a rename, and a base moves only on acknowledgement, so
/// the worst an abandoned round costs is a node offered again on the next one.
fn stopped() -> WobuError {
    WobuError::new(Code::Cancelled, "Sync is shutting down.")
}

/// The node ids this side now holds the peer's bytes for.
///
/// Three outcomes and no more. `Conflicted`, `AlreadyParked` and `AlreadyRefused`
/// are all "we have their version and it is not what is in the file", and
/// acknowledging one would let the peer record a base claiming we hold bytes we
/// pointedly do not — which the next round would honour as a licence to
/// fast-forward over the version a person chose to keep. `SendOurs` is the
/// opposite direction. `Deleted` and `Refused` wrote nothing.
fn agreed(report: &ApplyReport) -> Vec<Id> {
    report
        .outcomes
        .iter()
        .filter(|(_, applied)| {
            matches!(applied, Applied::FastForwarded { .. } | Applied::Converged | Applied::InStep)
        })
        .map(|(id, _)| *id)
        .collect()
}

fn absorb(half: &mut Half, report: &ApplyReport) {
    half.parked += report.parked().len();
    half.applied += report
        .outcomes
        .iter()
        .filter(|(_, applied)| {
            matches!(applied, Applied::FastForwarded { .. } | Applied::Conflicted { .. })
        })
        .count();
    half.changed |= report.changed_the_folder();

    for (id, why) in report.refusals() {
        // Worth a log line and nothing more. A refusal is per node and never
        // aborts a batch — one mangled file from one peer must not stop the
        // other two hundred good ones — and the strings inside are a stranger's.
        crate::diag::error(format!("sync: refused node {id}: {why:?}"));
    }
}

/// The blob half of a round, which is #81's.
///
/// It is a pair of functions rather than a `todo!()` or a missing call, and that
/// is deliberate: the manifest exchange is symmetric and takes both lists in one
/// call, so there is exactly one place blobs enter and exactly one place they
/// leave, and naming both now means #81 is a diff to two function bodies rather
/// than a change to the shape of a round.
///
/// **What this currently says on the wire is "I hold no blobs", and that is
/// safe** for precisely the reason `wobu-sync` writes down twice: an absence in
/// a manifest means "never had it", never "deleted it". A peer reading our empty
/// blob list concludes it has some files we do not and offers them; it does not
/// conclude we removed anything, and nothing anywhere deletes an asset because a
/// peer failed to mention it. The cost of the seam being empty is that assets do
/// not sync yet, which is the true state of M3.
mod blobs {
    use super::Blob;

    /// What this replica announces it holds.
    ///
    /// #81 fills this in by walking `assets/` and `generations/` and pairing
    /// each file with the BLAKE3 hex the index already has. It is not done here
    /// because hashing the asset tree on every round is a decision about I/O
    /// budget that belongs with the transfer that needs it.
    pub fn announce() -> Vec<Blob> {
        Vec::new()
    }

    /// What the peer announced.
    ///
    /// #81 turns this into fetches: a hash we do not have is a file we do not
    /// have. Every entry has already been through
    /// `wobu_sync::manifest::is_syncable_rel_path`, which is a *syntax* check
    /// and not a permission — the validation still has to happen on the near
    /// side of whatever join places the file, because a check performed in a
    /// different crate is one that stops being performed the day somebody adds a
    /// second caller.
    pub fn received(blobs: &[Blob]) {
        let _ = blobs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(outcomes: Vec<(Id, Applied)>) -> ApplyReport {
        ApplyReport { peer_id: "a".repeat(64), outcomes }
    }

    #[test]
    fn only_a_node_we_actually_hold_their_bytes_for_is_acknowledged() {
        // The rule the whole three-way compare rests on, from the acknowledging
        // side. A peer records a base from this list, and a base licenses a
        // fast-forward without asking anybody — so acknowledging a node whose
        // incoming version was parked beside ours rather than written would hand
        // the peer permission to overwrite the version a person chose to keep,
        // on the very next round.
        let ids: Vec<Id> = (1u128..=8).map(Id::from).collect();
        let report = report(vec![
            (ids[0], Applied::FastForwarded { rel_path: "nodes/character/kael.md".into() }),
            (ids[1], Applied::Converged),
            (ids[2], Applied::InStep),
            (ids[3], Applied::Conflicted { conflict_path: "nodes/character/kael.c.md".into() }),
            (ids[4], Applied::AlreadyParked { conflict_path: "nodes/character/kael.c.md".into() }),
            (ids[5], Applied::AlreadyRefused),
            (ids[6], Applied::SendOurs),
            (ids[7], Applied::Deleted),
        ]);

        assert_eq!(agreed(&report), vec![ids[0], ids[1], ids[2]]);
    }

    #[test]
    fn a_round_that_only_parked_conflicts_still_says_the_folder_moved() {
        // Two facts that look like one. A conflict sibling is a new file, so the
        // window has to refetch; but nobody's version was replaced and no base
        // moved, so the round achieved nothing that a repeat would not achieve
        // identically. Confusing the two is how a permanently conflicted pair
        // ends up polling hot for ever.
        let id = Id::from(1u128);
        let mut half = Half::default();
        absorb(
            &mut half,
            &report(vec![(
                id,
                Applied::Conflicted { conflict_path: "nodes/character/kael.c.md".into() },
            )]),
        );

        assert!(half.changed, "a new sibling is a folder the window has to reread");
        assert_eq!(half.parked, 1);

        let outcome = Outcome { applied: half.applied, changed: true, ..Outcome::default() };
        assert!(outcome.changed);
        assert!(
            !Outcome { parked: 1, changed: true, ..Outcome::default() }.did_something(),
            "a round that only re-parked a conflict must not reset the backoff"
        );
    }

    #[test]
    fn a_round_that_moved_bytes_in_either_direction_was_worth_the_dial() {
        assert!(Outcome { applied: 1, ..Outcome::default() }.did_something());
        assert!(Outcome { pushed: 1, ..Outcome::default() }.did_something());
        assert!(Outcome { served: 1, ..Outcome::default() }.did_something());
        assert!(!Outcome::default().did_something());
    }

    #[test]
    fn an_empty_blob_manifest_is_an_absence_and_absences_are_never_deletions() {
        // The seam #81 fills, pinned so that "we send nothing" stays a stated
        // position rather than an oversight somebody later reads as a bug. The
        // safety of it is `wobu-sync`'s rule, not this file's: a peer reading an
        // empty list concludes it has files we do not, and offers them.
        assert!(blobs::announce().is_empty());
        blobs::received(&[]);
    }
}
