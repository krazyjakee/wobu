//! What each peer has agreed to, and what it has been refused.
//!
//! Per-peer rather than global: two machines on one share are at different
//! points in the same history, and a single "last seen" would let a fast peer
//! speak for a slow one.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};
use wobu_core::Id;

use super::*;
use crate::error::Result;

impl Index {
    /// What this project and `peer_id` last agreed the node's bytes were.
    ///
    /// `None` is not an error and is the normal state for a peer that has never
    /// synced this node: it means "no base", which #80 reads as concurrent —
    /// the safe answer, because it can only ever cost a comparison or a
    /// conflict card, never an overwrite.
    pub fn base_hash(&self, peer_id: &str, node_id: Id) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT base_hash FROM sync_state WHERE peer_id = ?1 AND node_id = ?2",
                params![peer_id, node_id.to_string()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Every base agreed with one peer, in one query.
    ///
    /// The shape #79's manifest diff wants. It compares a whole manifest at
    /// once — a few hundred nodes — and asking [`base_hash`] per node would
    /// turn one indexed scan into a few hundred round trips for a table small
    /// enough to fit in a page or two.
    ///
    /// Rows whose `node_id` this build cannot parse are skipped, with the same
    /// forgiveness [`list_nodes`] shows: one unreadable row must cost that node
    /// a re-compare, not take the whole sync down.
    ///
    /// [`base_hash`]: Index::base_hash
    /// [`list_nodes`]: Index::list_nodes
    pub fn bases_for_peer(&self, peer_id: &str) -> Result<HashMap<Id, String>> {
        let mut stmt =
            self.conn.prepare("SELECT node_id, base_hash FROM sync_state WHERE peer_id = ?1")?;
        let rows = stmt.query_map(params![peer_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (node_id, base) = row?;
            let Ok(node_id) = Id::from_string(&node_id) else { continue };
            out.insert(node_id, base);
        }
        Ok(out)
    }

    /// Move the base for a batch of nodes, all or nothing.
    ///
    /// A batch rather than a row at a time because that is how agreement
    /// happens: #80 applies a work list and every node it settled — fast
    /// forwarded, sent, or found to have converged on identical bytes — moves
    /// its base at the same moment. Two things follow from doing it in one
    /// transaction.
    ///
    /// **A crash leaves a base set from one moment rather than a mixture.** Not
    /// because a partial set is unsafe — it is not; the un-moved rows simply
    /// re-compare next time — but because "these are the bases as of the end of
    /// that sync" is a sentence someone debugging a spurious conflict can
    /// reason about, and "some prefix of them" is not.
    ///
    /// **And it is one commit rather than N.** Outside a transaction every
    /// statement commits on its own, taking its own write lock and its own WAL
    /// frame; a few hundred of those to record a sync that has already finished
    /// is pure latency at the end of the operation the user is watching.
    ///
    /// Existing rows are overwritten. A base is the *last* thing agreed, not a
    /// history, and keeping the old value would be keeping a fact that both
    /// sides have already moved past.
    pub fn record_bases(&self, peer_id: &str, bases: &[(Id, String)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO sync_state (peer_id, node_id, base_hash) VALUES (?1,?2,?3)
                 ON CONFLICT(peer_id, node_id) DO UPDATE SET base_hash = excluded.base_hash",
            )?;
            for (node_id, base_hash) in bases {
                stmt.execute(params![peer_id, node_id.to_string(), base_hash])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The one-node case, for the paths that settle a single file — a conflict
    /// the user resolved, a save pushed to a peer that acknowledged it.
    ///
    /// Deliberately the same statement as [`record_bases`], rather than a
    /// second copy of the SQL that could drift from it.
    ///
    /// [`record_bases`]: Index::record_bases
    pub fn record_base(&self, peer_id: &str, node_id: Id, base_hash: &str) -> Result<()> {
        self.record_bases(peer_id, &[(node_id, base_hash.to_string())])
    }

    /// Forget everything agreed with a peer.
    ///
    /// For the moment the relationship ends rather than pauses: a share
    /// revoked, a ticket rotated, an identity that will not be dialled again.
    /// Keeping the rows would leave a base attributed to a peer that is not
    /// coming back, and if that `EndpointId` ever *did* return it would be
    /// trusted as a shared history neither side has any reason to still hold.
    ///
    /// The cost of being wrong about this is bounded and one-directional:
    /// forgetting too much re-compares, forgetting too little trusts. So this
    /// deletes rather than tombstones, and callers should reach for it whenever
    /// they are unsure.
    ///
    /// **Refusals go with the bases**, in the same transaction, and the same
    /// sentence explains why. A refusal is a fact about a *relationship* — this
    /// person, having seen who it came from, said no to that — so a relationship
    /// that has ended cannot leave one behind. If the id came back it would be a
    /// share re-granted or a ticket re-issued, and the first thing this project
    /// would do with a stale refusal is decline to show the user a version of
    /// their world that a machine they have just re-admitted is holding. That is
    /// the one way a row in this table can cost text rather than patience, and
    /// it is closed here.
    ///
    /// The asymmetry still holds in the other direction, too, which is what
    /// makes this an easy call: forgetting a refusal costs one redundant card,
    /// and only if the peer returns *and* still holds the exact bytes that were
    /// refused.
    pub fn forget_peer(&self, peer_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM sync_state WHERE peer_id = ?1", params![peer_id])?;
        tx.execute("DELETE FROM sync_rejected WHERE peer_id = ?1", params![peer_id])?;
        tx.commit()?;
        Ok(())
    }

    /* ── refusals ────────────────────────────────────────────────────────
     *
     * `sync_state` records what two machines agreed. This records what one
     * person refused. The table carries the argument for existing; these are the
     * three questions asked of it, and they are deliberately the same shape as
     * the three above so that the two halves of #89 read as one mechanism.
     *
     * A hash here is a *remote* hash — the bytes a peer sent and a human
     * declined — and it is the full BLAKE3 hex `crate::atomic::Stamp` holds, not
     * the truncated `source_version`. Mixing them would be worse here than
     * anywhere else in this file: `source_version` is blind to names and
     * summaries, so a refusal keyed on one would also suppress a rename the peer
     * made afterwards, which is the exact lost edit the third column exists to
     * prevent.
     */

    /// Record that a person refused one version of one node from one peer.
    ///
    /// Idempotent, because the button is. `DO NOTHING` rather than
    /// `DO UPDATE`: every column is in the primary key, so a second press of the
    /// same decision has nothing to say that the first did not.
    ///
    /// Singular where [`record_bases`] is plural, and that is not an oversight.
    /// Agreement settles in batches — a whole sync round reaches it at once —
    /// whereas a refusal is one person, one card, one press, and a batch API
    /// here would be an invitation to synthesise refusals from something other
    /// than a human decision. There is exactly one caller and it should stay
    /// that way.
    ///
    /// [`record_bases`]: Index::record_bases
    pub fn record_rejection(&self, peer_id: &str, node_id: Id, rejected_hash: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_rejected (peer_id, node_id, rejected_hash) VALUES (?1,?2,?3)
             ON CONFLICT(peer_id, node_id, rejected_hash) DO NOTHING",
            params![peer_id, node_id.to_string(), rejected_hash],
        )?;
        Ok(())
    }

    /// Every version this project refused from one peer, node by node, in one
    /// query.
    ///
    /// The shape [`crate::apply::apply`] wants, and the counterpart of
    /// [`bases_for_peer`]: a whole batch is compared at once, so asking per node
    /// would turn one indexed scan into a few hundred round trips over a table
    /// that fits in a page.
    ///
    /// The value is a *set* per node and not a single hash. That is the trap in
    /// this feature written into a type: a node can be refused more than once —
    /// two different paragraphs from the same peer, on two different days — and
    /// a shape that could only hold the latest would make the earlier refusal
    /// come back, which is the bug being fixed.
    ///
    /// Rows whose `node_id` this build cannot parse are skipped, with the same
    /// forgiveness [`bases_for_peer`] shows. One unreadable row costs that node
    /// a redundant conflict card, not the sync.
    ///
    /// [`bases_for_peer`]: Index::bases_for_peer
    pub fn rejections_for_peer(&self, peer_id: &str) -> Result<HashMap<Id, HashSet<String>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT node_id, rejected_hash FROM sync_rejected WHERE peer_id = ?1")?;
        let rows = stmt.query_map(params![peer_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out: HashMap<Id, HashSet<String>> = HashMap::new();
        for row in rows {
            let (node_id, hash) = row?;
            let Ok(node_id) = Id::from_string(&node_id) else { continue };
            out.entry(node_id).or_default().insert(hash);
        }
        Ok(out)
    }

    /// Whether one exact version of one node was refused from one peer.
    ///
    /// The single-row read, for callers holding one decision rather than a
    /// batch — and for tests, which is where the distinction between "this hash"
    /// and "this node" most needs asserting.
    pub fn is_rejected(&self, peer_id: &str, node_id: Id, hash: &str) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM sync_rejected
                 WHERE peer_id = ?1 AND node_id = ?2 AND rejected_hash = ?3",
                params![peer_id, node_id.to_string(), hash],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Every peer id this project has ever agreed or disagreed with.
    ///
    /// Exists for one caller — [`crate::Project::resolve_conflict`], which holds
    /// a conflict sibling's *alias* and needs the id behind it. An alias is
    /// derived from the id and there is no table mapping one back (#76's whole
    /// argument), so the only honest way to invert it is to re-derive it over a
    /// candidate set and see which candidate matches. This is that set.
    ///
    /// Deliberately sourced from both tables rather than from `sync_state`
    /// alone: once a peer has had a refusal recorded, it must stay resolvable
    /// even if every base with it is later replaced or dropped, or a second
    /// refusal for the same peer would land nowhere.
    pub fn known_peers(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_id FROM sync_state
             UNION
             SELECT peer_id FROM sync_rejected",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}
