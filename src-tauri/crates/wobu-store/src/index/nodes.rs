//! Nodes, their links, and the staleness the enhance pass reads.

use std::collections::{BTreeSet, HashMap, VecDeque};

use rusqlite::{OptionalExtension, params};
use wobu_core::{
    DescriptionState, EnhanceStamp, Id, LinkEdge, LinkRole, Node, NodeKind, NodeSummary, kind_def,
};

use super::*;
use crate::atomic::Stamp;
use crate::error::Result;

impl Index {
    pub fn upsert_node(&self, node: &Node, rel_path: &str, stamp: &Stamp) -> Result<()> {
        // A node can arrive at a path the index still attributes to someone
        // else — someone renamed files in Obsidian, or swapped two of them. Left
        // alone this trips the UNIQUE constraint on rel_path, and because
        // reconcile runs during `open`, that turns a rename into a project that
        // will not open at all. Evict the stale row; if its file still exists,
        // the same scan re-adds it at whatever path it now occupies.
        let tx = self.conn.unchecked_transaction()?;
        let displaced = {
            let mut statements = NodeWriteStatements::prepare(&tx, &self.write_metrics)?;
            statements.upsert_node(node, rel_path, stamp)?
        };
        tx.commit()?;
        self.write_metrics.committed();
        if let Some(displaced) = displaced {
            self.touch(displaced);
        }
        self.touch(node.id);
        Ok(())
    }

    pub fn remove_node(&self, id: Id) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut statements = NodeWriteStatements::prepare(&tx, &self.write_metrics)?;
            statements.remove_node(&id.to_string())?;
        }
        tx.commit()?;
        self.write_metrics.committed();
        self.touch(id);
        Ok(())
    }

    pub fn remove_by_rel_path(&self, rel_path: &str) -> Result<Option<Id>> {
        let id: Option<String> = self
            .conn
            .query_row("SELECT id FROM nodes WHERE rel_path = ?1", params![rel_path], |r| r.get(0))
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        let parsed = Id::from_string(&id).ok();
        if let Some(parsed) = parsed {
            self.remove_node(parsed)?;
        }
        Ok(parsed)
    }

    /// `(id, name)` for whatever node lives at a path.
    ///
    /// The reverse of [`rel_path_of`](Self::rel_path_of), and the lookup a
    /// conflict sibling needs: the card knows the filename it was parked beside
    /// and has to say "Kael Vantris" rather than quote a path at the user.
    pub fn node_at_rel_path(&self, rel_path: &str) -> Result<Option<(Id, String)>> {
        let row: Option<(String, String)> = self
            .conn
            .query_row("SELECT id, name FROM nodes WHERE rel_path = ?1", params![rel_path], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row.and_then(|(id, name)| Id::from_string(&id).ok().map(|id| (id, name))))
    }

    /// The navigator's data source. Ordered so the tree renders without a sort:
    /// registry order by kind, then alphabetically.
    ///
    /// `description_state` here is the **effective** state, not the stored one:
    /// a node whose file says `fresh` or `edited` is reported `stale` when its
    /// [`EnhanceStamp`] no longer matches the world. That overlay is why an
    /// edit to the Style Guide can put a dot on a hundred rows without a single
    /// file being written — and why it is confined to this summary, which is a
    /// view. [`node`](Self::node) and `Project::get_node` still answer with what
    /// the Markdown says, because those are what get saved back, and writing a
    /// derived `stale` over a stored `edited` is exactly the data loss the
    /// derivation exists to avoid.
    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>> {
        let stale = self.stale_nodes()?;
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, slug, summary, parent_id, description_state
             FROM nodes ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], NodeSummaryRow::decode)?;

        let mut out = Vec::new();
        for row in rows {
            let row = row?;
            let Some(summary) = row.into_summary(&stale) else {
                // A row we cannot parse means the index is out of step with the
                // build; skipping it keeps the tree usable until the rebuild.
                continue;
            };
            out.push(summary);
        }

        let order = |k: NodeKind| {
            wobu_core::kind_registry().iter().position(|d| d.kind == k).unwrap_or(usize::MAX)
        };
        out.sort_by_key(|n| (order(n.kind), n.name.to_lowercase()));
        Ok(out)
    }

    /// Every node in the project, whole, ordered by id.
    ///
    /// The influence engine's input. It borrows already-loaded `Node`s and does
    /// no IO of its own, deliberately, so that `prompt_compile` stays
    /// sub-millisecond (`wobu_influence::World`); this is where those nodes come
    /// from, and it reads local disk rather than the project folder. That is the
    /// whole point — the alternative is a read of every Markdown file, over
    /// whatever the world is mounted on, on every Inspector interaction.
    ///
    /// Ordered by id because `World` picks the Style Guide by lowest id and a
    /// caller must not be able to change which node that is by handing them over
    /// in a different order. Rows this build cannot parse are skipped with the
    /// same forgiveness [`list_nodes`](Self::list_nodes) shows: one entity
    /// missing from a stack is survivable, a panel that will not open is not.
    pub fn nodes(&self) -> Result<Vec<Node>> {
        let mut stmt = self.conn.prepare("SELECT doc FROM nodes ORDER BY id")?;
        collect_json_documents(stmt.query_map([], document_row)?)
    }

    /// One node, whole, without opening its file.
    ///
    /// What [`crate::Project::world_nodes`] re-reads when a single row moves —
    /// the reason a save, or a collaborator's edit arriving through `reconcile`,
    /// does not cost a query over the entire world.
    pub fn node(&self, id: Id) -> Result<Option<Node>> {
        let doc: Option<String> = self
            .conn
            .query_row("SELECT doc FROM nodes WHERE id = ?1", params![id.to_string()], |r| r.get(0))
            .optional()?;
        Ok(doc.and_then(|doc| serde_json::from_str(&doc).ok()))
    }

    /* ── staleness ───────────────────────────────────────────────────────
     *
     * `description_state = stale` is derived rather than stored, and this is
     * where it is derived. The argument for that lives on
     * `wobu_core::DescriptionState`; what follows is the mechanics.
     */

    /// Every node whose description no longer matches what it was enhanced
    /// from.
    ///
    /// One query over a handful of narrow columns and no folder access at all,
    /// which is what makes an edit to the Style Guide cheap: it invalidates most of
    /// the project, and the cost of saying so is recomputing this — not
    /// rewriting a hundred Markdown files, moving a hundred `updated_at`
    /// stamps, and giving a hundred guarded writes the chance to lose a race
    /// with a collaborator.
    ///
    /// A node is stale when any of these holds:
    ///
    /// - its file *says* `stale`, which only an older Wobu or a person editing
    ///   frontmatter can have written, and which is honoured rather than
    ///   second-guessed;
    /// - its own notes, attributes or edges have moved since it was stamped;
    /// - a source it was enhanced from now hashes differently, or has been
    ///   deleted, so the layer it contributed is gone.
    ///
    /// A `fresh` or `edited` description with **no** stamp is deliberately not
    /// stale. That is every description written before stamps existed and every
    /// one typed straight into the Markdown, and there is nothing to compare
    /// them against — a dot on every row of an existing project is the noise
    /// that teaches people to ignore the dot.
    pub fn stale_nodes(&self) -> Result<BTreeSet<Id>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description_state, subject_version, source_version, enhanced_from
             FROM nodes",
        )?;
        let rows = stmt.query_map([], StalenessRow::decode)?;

        // Collected whole first, because a node's answer depends on other
        // rows' `source_version` and a streaming pass would have to re-query
        // per source — the shape that turns one scan into a hundred.
        let mut current: HashMap<Id, String> = HashMap::new();
        let mut candidates = Vec::new();
        for row in rows {
            let row = row?;
            let Ok(id) = Id::from_string(&row.id) else { continue };
            current.insert(id, row.source);
            candidates.push((id, row.state, row.subject, row.stamp));
        }

        let mut stale = BTreeSet::new();
        for (id, state, subject, stamp) in candidates {
            let state: DescriptionState =
                serde_json::from_value(serde_json::Value::String(state)).unwrap_or_default();
            match state {
                DescriptionState::Stale => {
                    stale.insert(id);
                    continue;
                }
                DescriptionState::Fresh | DescriptionState::Edited => {}
                // `none` has nothing to invalidate, and `enhancing` is being
                // rewritten as we speak — offering a re-enhance for a job that
                // is already running is worse than saying nothing.
                DescriptionState::None | DescriptionState::Enhancing => continue,
            }

            let Ok(stamp) = serde_json::from_str::<EnhanceStamp>(&stamp) else { continue };
            let moved = stamp.subject != subject
                || stamp
                    .sources
                    .iter()
                    .any(|s| current.get(&s.node).map(String::as_str) != Some(&s.version));
            if moved {
                stale.insert(id);
            }
        }
        Ok(stale)
    }

    /// Every node whose influence stack contains `id` — the nodes an edit to it
    /// invalidates.
    ///
    /// The downstream direction of `wobu_influence::resolve`, and deliberately
    /// the same graph read backwards rather than a second opinion about what
    /// "upstream" means: `parent_id` is an implicit edge, disabled links
    /// contribute nothing, and `related_to` is a lateral nod that resolves to a
    /// source but is never expanded through. That last rule is the only one
    /// that needs care in reverse. A path reaches `id` only if every node along
    /// it was expanded, and a node reached by `related_to` is not — so the first
    /// reverse hop may cross any role, and every hop after it may not cross
    /// `related_to`.
    ///
    /// The two singletons are answered without walking. Nothing links to the
    /// Style Guide; it is a root of *every* stack in the project
    /// (`docs/04-influence-engine.md`), so a walk over the edges would find
    /// nobody and report that editing it changed nothing at all.
    ///
    /// Cycles terminate the same way they do upstream — first visit wins, so
    /// the walk is bounded by the number of nodes rather than by a hop counter.
    /// A world with a link loop is something a user can create in two clicks
    /// and something Obsidian can create by hand, so this cannot be left to a
    /// depth limit.
    pub fn dependents_of(&self, id: Id) -> Result<BTreeSet<Id>> {
        if let Some((kind, _)) = self.kind_and_parent(id)?
            && kind_def(kind).singleton
        {
            let mut all = self.all_ids()?;
            all.remove(&id);
            return Ok(all);
        }

        let mut out = BTreeSet::new();
        let mut visited = BTreeSet::from([id]);
        let mut queue = VecDeque::from([id]);

        while let Some(current) = queue.pop_front() {
            for from in self.referrers(current, current == id)? {
                if visited.insert(from) {
                    out.insert(from);
                    queue.push_back(from);
                }
            }
        }
        Ok(out)
    }

    /// Nodes with an edge *into* `id`: children by nesting, plus the sources of
    /// enabled links. `lateral` admits `related_to`, which only the first hop
    /// of a downstream walk may do.
    fn referrers(&self, id: Id, lateral: bool) -> Result<Vec<Id>> {
        // The excluded role comes from `LinkRole` rather than being spelled
        // into the SQL, so that renaming it in `wobu-core` cannot leave this
        // silently matching nothing and quietly walking through laterals.
        let excluded = if lateral { "" } else { LinkRole::RelatedTo.as_str() };
        let mut stmt = self
            .conn
            .prepare("SELECT from_id FROM links WHERE to_id = ?1 AND enabled = 1 AND role <> ?2")?;
        let rows = stmt.query_map(params![id.to_string(), excluded], |r| r.get::<_, String>(0))?;
        let mut out: Vec<Id> =
            rows.filter_map(|r| r.ok().and_then(|s| Id::from_string(&s).ok())).collect();
        out.extend(self.children_of(id)?);
        Ok(out)
    }

    fn all_ids(&self) -> Result<BTreeSet<Id>> {
        let mut stmt = self.conn.prepare("SELECT id FROM nodes")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok().and_then(|s| Id::from_string(&s).ok())).collect())
    }

    /* ── assets ──────────────────────────────────────────────────────────
     *
     * Nothing here is canonical either. Every column comes off the file — the
     * hash and the id off its name, the rest off its header — so this table is
     * a cache of a directory listing, and `crate::assets::scan` refills it.
     */

    pub fn rel_path_of(&self, id: Id) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT rel_path FROM nodes WHERE id = ?1", params![id.to_string()], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn stamp_of(&self, id: Id) -> Result<Option<Stamp>> {
        Ok(self
            .conn
            .query_row(
                "SELECT mtime_ms, size, hash FROM nodes WHERE id = ?1",
                params![id.to_string()],
                |r| {
                    Ok(Stamp {
                        mtime_ms: r.get(0)?,
                        size: r.get::<_, i64>(1)? as u64,
                        hash: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// `node_id -> content hash` for every indexed node: this project's half of
    /// a manifest.
    ///
    /// Full BLAKE3 hex, the same string `sync_state` holds and the same one
    /// [`stamp_of`] returns — so a manifest and a base are directly comparable,
    /// which is the entire point of the column. Not `source_version`, which is
    /// deliberately blind to names and summaries; see the note on `sync_state`.
    ///
    /// This reports what the *index* last read, which is one reconcile behind
    /// the folder. That staleness is fine here and only here: a manifest is a
    /// pre-filter that decides which bytes are worth asking for, and
    /// [`crate::apply::apply`] re-reads the disk before it writes anything. The
    /// alternative — hashing every node file over SMB to answer a poll — is the
    /// cost this index exists to avoid.
    ///
    /// [`stamp_of`]: Index::stamp_of
    pub fn node_hashes(&self) -> Result<HashMap<Id, String>> {
        let mut stmt = self.conn.prepare("SELECT id, hash FROM nodes")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, hash) = row?;
            // One unreadable id costs that node a re-compare rather than taking
            // the whole sync down, matching `bases_for_peer` and `list_nodes`.
            let Ok(id) = Id::from_string(&id) else { continue };
            out.insert(id, hash);
        }
        Ok(out)
    }

    /// `rel_path -> (mtime, size)` for every indexed node. The reconciler
    /// compares a directory listing against this and only re-reads files whose
    /// stamp moved — the asymmetry the whole index exists for.
    pub fn all_stamps(&self) -> Result<HashMap<String, (i64, u64)>> {
        let mut stmt = self.conn.prepare("SELECT rel_path, mtime_ms, size FROM nodes")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (path, mtime, size) = row?;
            out.insert(path, (mtime, size as u64));
        }
        Ok(out)
    }

    /// `(kind, parent_id)` — what [`wobu_core::validate_parent`] needs to detect
    /// cycles without loading whole nodes.
    pub fn kind_and_parent(&self, id: Id) -> Result<Option<(NodeKind, Option<Id>)>> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT kind, parent_id FROM nodes WHERE id = ?1",
                params![id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(kind, parent)| {
            kind.parse::<NodeKind>()
                .ok()
                .map(|k| (k, parent.and_then(|p| Id::from_string(&p).ok())))
        }))
    }

    pub fn children_of(&self, id: Id) -> Result<Vec<Id>> {
        let mut stmt = self.conn.prepare("SELECT id FROM nodes WHERE parent_id = ?1")?;
        let rows = stmt.query_map(params![id.to_string()], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok().and_then(|s| Id::from_string(&s).ok())).collect())
    }

    pub fn singleton_of(&self, kind: NodeKind) -> Result<Option<Id>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM nodes WHERE kind = ?1 LIMIT 1",
                params![kind.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.and_then(|s| Id::from_string(&s).ok()))
    }

    pub fn slugs_in_kind(&self, kind: NodeKind) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT slug FROM nodes WHERE kind = ?1")?;
        let rows = stmt.query_map(params![kind.as_str()], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Everything pointing *at* this node — "3 characters inherit from this".
    pub fn backlinks(&self, id: Id) -> Result<Vec<LinkEdge>> {
        let mut stmt = self
            .conn
            .prepare("SELECT from_id, to_id, role, weight, enabled FROM links WHERE to_id = ?1")?;
        collect_links(stmt.query_map(params![id.to_string()], LinkRow::decode)?)
    }

    /// Every explicit influence edge in the world.
    ///
    /// The relationship map is a project-wide view, so asking `backlinks` once
    /// per node would turn one local-index read into hundreds of queries. The
    /// index already owns the complete edge table; return it in one pass.
    pub fn links(&self) -> Result<Vec<LinkEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT from_id, to_id, role, weight, enabled
             FROM links ORDER BY from_id, to_id, role",
        )?;
        collect_links(stmt.query_map([], LinkRow::decode)?)
    }

    /// Full-text search over names, summaries, notes and descriptions.
    pub fn search(&self, query: &str) -> Result<Vec<Id>> {
        let Some(expr) = fts_match_expr(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM node_fts WHERE node_fts MATCH ?1 ORDER BY rank LIMIT 200")?;
        let rows = stmt.query_map(params![expr], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok().and_then(|s| Id::from_string(&s).ok())).collect())
    }

    /* ── sync state ──────────────────────────────────────────────────────
     *
     * One fact per (peer, node): the hash both sides last agreed on. The
     * argument for the table is on the table; these are the four questions M3
     * asks of it.
     *
     * A hash here is the same string `crate::atomic::Stamp` holds — full BLAKE3
     * hex over the file's bytes — and *not* the truncated `source_version`
     * further down this file. The two must not be confused: a version is
     * deliberately blind to names, summaries and link weights, and a sync that
     * ignored a rename would be a sync that loses one.
     *
     * Nothing here is keyed to a node existing. `Index::remove_node` does not
     * clear bases and must not start: it runs during reconcile for files that
     * have merely gone missing — a share half-mounted, a sync client mid-write
     * — and a folder blinking must not be able to reset what a peer was known
     * to hold. Ids are ULIDs and never reused, so a base left behind by a node
     * that really was deleted can only ever be read back for the node it was
     * written for.
     *
     * The same is true of the refusals below, and it is worth walking rather
     * than assuming, because the two tables are not obviously alike. A base
     * survives `remove_node` so that a blinking folder cannot *destroy*
     * agreement; a refusal survives it so that a blinking folder cannot
     * *resurrect* a card. Those pull in opposite directions and both land on
     * "leave the row alone", which is the answer that costs nothing either way:
     * a leftover refusal is three strings keyed to a ULID that will never be
     * issued again, and it can only ever match the node it was written for and
     * the exact bytes that were refused.
     */
}
