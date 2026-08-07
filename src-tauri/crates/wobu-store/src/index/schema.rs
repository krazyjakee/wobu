//! The schema, and the prepared statements the hot paths reuse.
//!
//! Preparing is not free, and a rebuild runs the same ten statements once per
//! node. `NodeWriteStatements` prepares them once per transaction instead, and
//! `WriteMetrics` is what the tests assert that against — a regression here is
//! invisible except as a slow open.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Statement, params};
use wobu_core::{Asset, Generation, Id, Node};

use super::version::description_text;
use super::*;
use crate::atomic::Stamp;
use crate::error::Result;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nodes (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL,
    name              TEXT NOT NULL,
    slug              TEXT NOT NULL,
    summary           TEXT NOT NULL DEFAULT '',
    parent_id         TEXT,
    description_state TEXT NOT NULL DEFAULT 'none',
    cover_asset_id    TEXT,
    rel_path          TEXT NOT NULL UNIQUE,
    mtime_ms          INTEGER NOT NULL DEFAULT 0,
    size              INTEGER NOT NULL DEFAULT 0,
    hash              TEXT NOT NULL DEFAULT '',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    -- The whole node as JSON. Derived like every other column here — it is what
    -- the Markdown parsed to, and a rebuild restores it — but it is the only one
    -- that hands back a `Node` rather than a summary of one, and a `Node` is
    -- what the influence engine takes.
    --
    -- Without it, building the `World` for a stack means reading every node file
    -- in the project, over whatever the folder is mounted on, on every drag of a
    -- weight slider. With it, `prompt_compile` never touches the folder at all:
    -- it reads from local disk, so it answers at the same speed whether the
    -- share is fast, slow, or currently unplugged. See `Index::nodes`.
    doc               TEXT NOT NULL DEFAULT '',

    -- The three columns staleness is derived from, all off the same Markdown
    -- as everything else here. They exist as columns rather than being read out
    -- of `doc` because the navigator asks the staleness question about every
    -- node at once, on every refresh: answering it from `doc` means parsing the
    -- whole world's JSON to look at a few dozen bytes of each node, which is
    -- the cost `Index::nodes` documents and the navigator has no reason to pay.
    --
    -- See `source_version`, `subject_version` and `Index::stale_nodes`.
    source_version    TEXT NOT NULL DEFAULT '',
    subject_version   TEXT NOT NULL DEFAULT '',
    enhanced_from     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS nodes_kind    ON nodes(kind);
CREATE INDEX IF NOT EXISTS nodes_parent  ON nodes(parent_id);

CREATE TABLE IF NOT EXISTS links (
    from_id TEXT NOT NULL,
    to_id   TEXT NOT NULL,
    role    TEXT NOT NULL,
    weight  REAL NOT NULL DEFAULT 1.0,
    enabled INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (from_id, to_id, role)
);
CREATE INDEX IF NOT EXISTS links_to ON links(to_id);

CREATE VIRTUAL TABLE IF NOT EXISTS node_fts USING fts5(
    id UNINDEXED, name, summary, notes, description, tokenize = 'unicode61'
);

-- Blobs under `assets/originals/`. Every column is derived from the file, so
-- this table is rebuildable from the folder like the rest of the index — see
-- `crate::assets::scan`. `hash` is UNIQUE because the id is derived from it:
-- two rows differing only in id is a state that cannot exist.
CREATE TABLE IF NOT EXISTS assets (
    id         TEXT PRIMARY KEY,
    hash       TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL,
    rel_path   TEXT NOT NULL UNIQUE,
    thumb_path TEXT,
    mime       TEXT NOT NULL,
    width      INTEGER NOT NULL,
    height     INTEGER NOT NULL,
    bytes      INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

-- Reference images attached to nodes, with the role that decides where each is
-- routed. Canonically these live in node frontmatter, exactly like `links`;
-- this table is refilled from there by `Index::upsert_node`, so a rebuild
-- restores it without reading anything else.
--
-- Indexed on `(node_id, role)` because that is the question M5's per-role image
-- budget asks once per role per layer while compiling: a five-layer stack over
-- seven roles is thirty-five lookups, and answering them by opening node files
-- over SMB is exactly what this index exists to avoid.
CREATE TABLE IF NOT EXISTS asset_links (
    node_id  TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    role     TEXT NOT NULL,
    weight   REAL NOT NULL DEFAULT 1.0,
    enabled  INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (node_id, asset_id, role)
);
CREATE INDEX IF NOT EXISTS asset_links_role  ON asset_links(node_id, role);
CREATE INDEX IF NOT EXISTS asset_links_asset ON asset_links(asset_id);

-- Append-only JSON under `generations/YYYY-MM/`. `doc` is the whole record so
-- the Concepts grid can open a result without touching the share; the other
-- columns are only the fields used to select and order its tiles. Every byte is
-- rebuildable from the JSON file, including provider/model/params and the full
-- influence snapshot, so this remains a disposable cache.
CREATE TABLE IF NOT EXISTS generations (
    id         TEXT PRIMARY KEY,
    node_id    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    preset     TEXT NOT NULL,
    view_type  TEXT,
    backend    TEXT NOT NULL,
    model      TEXT NOT NULL,
    seed       INTEGER NOT NULL,
    prompt_excerpt TEXT NOT NULL,
    first_asset_id TEXT,
    output_count INTEGER NOT NULL DEFAULT 0,
    seed_source TEXT,
    used_locked_seed INTEGER,
    scene_subject_names TEXT NOT NULL DEFAULT '[]',
    rel_path   TEXT NOT NULL UNIQUE,
    mtime_ms   INTEGER NOT NULL DEFAULT 0,
    size       INTEGER NOT NULL DEFAULT 0,
    hash       TEXT NOT NULL DEFAULT '',
    doc        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS generations_node ON generations(node_id, created_at DESC, id DESC);

-- Files that are on disk and could not be parsed. Deliberately a table of its
-- own rather than a column on `nodes`: a file a sync client truncated may never
-- have had a node row at all, and the ones that did must keep it. See
-- `Index::mark_corrupt`.
CREATE TABLE IF NOT EXISTS corrupt (
    rel_path    TEXT PRIMARY KEY,
    node_id     TEXT,
    error       TEXT NOT NULL,
    detected_at TEXT NOT NULL
);

-- The last content hash this project and one peer agreed on for one node: the
-- base of the three-way compare, and the whole of what M3 needs to tell "we
-- both changed it" from "one of us did". Everything else the compare wants —
-- our hash now, theirs now — is already known at the moment of comparing.
--
-- The alternative was a version vector in the frontmatter, and it is worth
-- writing down why not. That would put sync bookkeeping inside files people
-- open in Obsidian, where an external edit changes the bytes and silently fails
-- to bump the vector, and a vector that lies is worse than no vector at all.
-- Content hashes are already this codebase's answer to "did these bytes move",
-- and no text editor can desynchronise one.
--
-- This is the only table here that is not derived from the folder: a rescan can
-- rebuild every other row and cannot reconstruct what a peer once held. It is
-- still safe to lose, which is the bar the module doc actually sets — with no
-- base every node reads as concurrent, so the cost is a full re-compare and a
-- run of conflict cards nobody needed, not a lost edit. That is why the version
-- bump above is still allowed to drop it along with everything else, and why
-- nothing here is ever written to the project folder.
--
-- `peer_id` is TEXT and stays TEXT. It is an iroh `EndpointId` (#76) rendered
-- as hex; storing it as itself would make opening a project depend on the
-- transport crate, and the index has to open on a machine that never syncs.
--
-- No separate index on `peer_id`: it is the leftmost column of the primary key,
-- so the per-peer read that #79's manifest diff makes already has one.
CREATE TABLE IF NOT EXISTS sync_state (
    peer_id     TEXT NOT NULL,
    node_id     TEXT NOT NULL,
    base_hash   TEXT NOT NULL,
    PRIMARY KEY (peer_id, node_id)
);

-- One version of one node, from one peer, that a person looked at and said no
-- to. The other half of `sync_state` above, and the reason it needs a table of
-- its own rather than a column beside `base_hash`.
--
-- A conflict deliberately does *not* move the base (#80, and the argument is in
-- `crate::apply`): moving it to the remote hash would stop the re-compare, which
-- is exactly what makes it tempting, but it claims agreement on bytes nobody
-- agreed to and the base is precisely what a later fast-forward trusts. So the
-- base stays put, the disagreement is rediscovered every round, and a card the
-- user has already dismissed comes straight back (#89). Agreement and refusal
-- are two different facts about the same pair of machines, and the only way to
-- record the second without corrupting the first is to record it separately.
--
-- **`rejected_hash` is in the primary key, and that is the whole of the safety
-- argument.** A two-column `(peer_id, node_id)` version of this table is the
-- obvious one and it silently eats edits: the user rejects Tuesday's paragraph,
-- the peer writes a better one on Wednesday, and Wednesday never appears on this
-- machine at all — no card, no file, no trace. Keyed on the bytes, a refusal
-- suppresses exactly the version that was refused and nothing else. Every later
-- version from that same peer is a new hash and conflicts as loudly as it should.
--
-- Rows accumulate and are never swept. One row is three short strings, a person
-- can only make them by pressing a button, and the alternative — expiring them
-- on a timer or a count — is a card that reappears weeks later for a decision
-- already made, which is the bug this table exists to fix arriving late.
--
-- Not derived from the folder, exactly like `sync_state`, and safe to lose for
-- the same reason: with the table gone every refusal is forgotten, the next sync
-- parks one redundant sibling per rejected node, and a person dismisses a card
-- they had dismissed before. That is a cost in patience and not in text, which is
-- why the version bump above is allowed to drop this along with everything else.
--
-- Nothing here may ever *cause* a write. A row can only ever turn a `Conflict`
-- into "do nothing" — see `crate::apply::already_refused`. If it could reach any
-- other arm of the table, a corrupt or forged row would become an overwrite.
CREATE TABLE IF NOT EXISTS sync_rejected (
    peer_id       TEXT NOT NULL,
    node_id       TEXT NOT NULL,
    rejected_hash TEXT NOT NULL,
    PRIMARY KEY (peer_id, node_id, rejected_hash)
);
"#;

pub(super) const UPSERT_NODE_SQL: &str = "INSERT INTO nodes
       (id, kind, name, slug, summary, parent_id, description_state, cover_asset_id,
        rel_path, mtime_ms, size, hash, created_at, updated_at, doc,
        source_version, subject_version, enhanced_from)
     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
     ON CONFLICT(id) DO UPDATE SET
       kind=excluded.kind, name=excluded.name, slug=excluded.slug,
       summary=excluded.summary, parent_id=excluded.parent_id,
       description_state=excluded.description_state,
       cover_asset_id=excluded.cover_asset_id, rel_path=excluded.rel_path,
       mtime_ms=excluded.mtime_ms, size=excluded.size, hash=excluded.hash,
       created_at=excluded.created_at, updated_at=excluded.updated_at,
       doc=excluded.doc, source_version=excluded.source_version,
       subject_version=excluded.subject_version,
       enhanced_from=excluded.enhanced_from";

pub(super) const UPSERT_ASSET_SQL: &str = "INSERT OR REPLACE INTO assets
       (id, hash, kind, rel_path, thumb_path, mime, width, height, bytes, created_at)
     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)";

pub(super) const UPSERT_GENERATION_SQL: &str = "INSERT INTO generations
       (id, node_id, created_at, preset, view_type, backend, model, seed,
        prompt_excerpt, first_asset_id, output_count, seed_source, used_locked_seed,
        scene_subject_names, rel_path, mtime_ms, size, hash, doc)
     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
     ON CONFLICT(id) DO UPDATE SET
       node_id=excluded.node_id, created_at=excluded.created_at,
       preset=excluded.preset, view_type=excluded.view_type,
       backend=excluded.backend, model=excluded.model, seed=excluded.seed,
       prompt_excerpt=excluded.prompt_excerpt, first_asset_id=excluded.first_asset_id,
       output_count=excluded.output_count, seed_source=excluded.seed_source,
       used_locked_seed=excluded.used_locked_seed,
       scene_subject_names=excluded.scene_subject_names,
       rel_path=excluded.rel_path, mtime_ms=excluded.mtime_ms,
       size=excluded.size, hash=excluded.hash, doc=excluded.doc";

pub(super) const CLEAR_DERIVED_SQL: &str =
    "DELETE FROM nodes; DELETE FROM links; DELETE FROM asset_links;
     DELETE FROM node_fts; DELETE FROM assets; DELETE FROM generations;
     DELETE FROM corrupt;";

#[cfg(test)]
pub(super) const NODE_WRITE_STATEMENT_COUNT: usize = 10;

#[cfg(test)]
pub(super) const REBUILD_STATEMENT_COUNT: usize = NODE_WRITE_STATEMENT_COUNT + 4;

pub(super) struct NodeWriteStatements<'conn> {
    displaced: Statement<'conn>,
    upsert_node: Statement<'conn>,
    delete_node: Statement<'conn>,
    delete_links_from: Statement<'conn>,
    delete_links_to: Statement<'conn>,
    insert_link: Statement<'conn>,
    delete_asset_links: Statement<'conn>,
    insert_asset_link: Statement<'conn>,
    delete_fts: Statement<'conn>,
    insert_fts: Statement<'conn>,
}

impl<'conn> NodeWriteStatements<'conn> {
    pub(super) fn prepare(conn: &'conn Connection, metrics: &WriteMetrics) -> Result<Self> {
        Ok(Self {
            displaced: prepare_statement(
                conn,
                "SELECT id FROM nodes WHERE rel_path = ?1 AND id <> ?2",
                metrics,
            )?,
            upsert_node: prepare_statement(conn, UPSERT_NODE_SQL, metrics)?,
            delete_node: prepare_statement(conn, "DELETE FROM nodes WHERE id = ?1", metrics)?,
            delete_links_from: prepare_statement(
                conn,
                "DELETE FROM links WHERE from_id = ?1",
                metrics,
            )?,
            delete_links_to: prepare_statement(
                conn,
                "DELETE FROM links WHERE to_id = ?1",
                metrics,
            )?,
            insert_link: prepare_statement(
                conn,
                "INSERT OR REPLACE INTO links (from_id, to_id, role, weight, enabled)
                 VALUES (?1,?2,?3,?4,?5)",
                metrics,
            )?,
            delete_asset_links: prepare_statement(
                conn,
                "DELETE FROM asset_links WHERE node_id = ?1",
                metrics,
            )?,
            insert_asset_link: prepare_statement(
                conn,
                "INSERT OR REPLACE INTO asset_links (node_id, asset_id, role, weight, enabled)
                 VALUES (?1,?2,?3,?4,?5)",
                metrics,
            )?,
            delete_fts: prepare_statement(conn, "DELETE FROM node_fts WHERE id = ?1", metrics)?,
            insert_fts: prepare_statement(
                conn,
                "INSERT INTO node_fts (id, name, summary, notes, description)
                 VALUES (?1,?2,?3,?4,?5)",
                metrics,
            )?,
        })
    }

    pub(super) fn remove_node(&mut self, id: &str) -> Result<()> {
        self.delete_node.execute(params![id])?;
        self.delete_links_from.execute(params![id])?;
        self.delete_links_to.execute(params![id])?;
        self.delete_asset_links.execute(params![id])?;
        self.delete_fts.execute(params![id])?;
        Ok(())
    }

    pub(super) fn upsert_node(
        &mut self,
        node: &Node,
        rel_path: &str,
        stamp: &Stamp,
    ) -> Result<Option<Id>> {
        let id = node.id.to_string();
        let displaced: Option<String> =
            self.displaced.query_row(params![rel_path, id], |row| row.get(0)).optional()?;
        let displaced = displaced.as_deref().and_then(|value| Id::from_string(value).ok());
        if let Some(displaced) = displaced {
            self.remove_node(&displaced.to_string())?;
        }

        let description_state =
            serde_json::to_value(node.description_state)?.as_str().unwrap_or("none").to_string();
        let doc = serde_json::to_string(node)?;
        let enhanced_from = match &node.enhanced_from {
            Some(stamp) => serde_json::to_string(stamp)?,
            None => String::new(),
        };
        self.upsert_node.execute(params![
            id,
            node.kind.as_str(),
            node.name,
            node.slug,
            node.summary,
            node.parent_id.map(|parent| parent.to_string()),
            description_state,
            node.cover_asset_id.map(|asset| asset.to_string()),
            rel_path,
            stamp.mtime_ms,
            stamp.size as i64,
            stamp.hash,
            node.created_at.to_rfc3339(),
            node.updated_at.to_rfc3339(),
            doc,
            source_version(node),
            subject_version(node),
            enhanced_from,
        ])?;

        self.delete_links_from.execute(params![id])?;
        for link in &node.links {
            self.insert_link.execute(params![
                id,
                link.to_id.to_string(),
                link.role.as_str(),
                link.weight,
                link.enabled as i32,
            ])?;
        }

        self.delete_asset_links.execute(params![id])?;
        for link in &node.asset_links {
            self.insert_asset_link.execute(params![
                id,
                link.asset_id.to_string(),
                link.role.as_str(),
                link.weight,
                link.enabled as i32,
            ])?;
        }

        self.delete_fts.execute(params![id])?;
        self.insert_fts.execute(params![
            id,
            node.name,
            node.summary,
            node.notes_raw,
            description_text(node),
        ])?;
        Ok(displaced)
    }
}

pub(super) struct RebuildStatements<'conn> {
    pub(super) nodes: NodeWriteStatements<'conn>,
    pub(super) upsert_asset: Statement<'conn>,
    pub(super) upsert_generation: Statement<'conn>,
    pub(super) corrupt_node_id: Statement<'conn>,
    pub(super) upsert_corrupt: Statement<'conn>,
}

impl<'conn> RebuildStatements<'conn> {
    pub(super) fn prepare(conn: &'conn Connection, metrics: &WriteMetrics) -> Result<Self> {
        Ok(Self {
            nodes: NodeWriteStatements::prepare(conn, metrics)?,
            upsert_asset: prepare_statement(conn, UPSERT_ASSET_SQL, metrics)?,
            upsert_generation: prepare_statement(conn, UPSERT_GENERATION_SQL, metrics)?,
            corrupt_node_id: prepare_statement(
                conn,
                "SELECT id FROM nodes WHERE rel_path = ?1",
                metrics,
            )?,
            upsert_corrupt: prepare_statement(
                conn,
                "INSERT INTO corrupt (rel_path, node_id, error, detected_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(rel_path) DO UPDATE SET node_id = ?2, error = ?3",
                metrics,
            )?,
        })
    }

    pub(super) fn upsert_asset(&mut self, asset: &Asset) -> Result<()> {
        self.upsert_asset.execute(params![
            asset.id.to_string(),
            asset.hash,
            serde_json::to_value(asset.kind)?.as_str().unwrap_or("reference"),
            asset.rel_path,
            asset.thumb_path,
            asset.mime,
            asset.width,
            asset.height,
            asset.bytes as i64,
            asset.created_at.to_rfc3339(),
        ])?;
        Ok(())
    }

    pub(super) fn upsert_generation(
        &mut self,
        generation: &Generation,
        rel_path: &str,
        stamp: &Stamp,
    ) -> Result<()> {
        let row = GenerationRow::new(generation, rel_path, stamp)?;
        self.upsert_generation.execute(&row.params()[..])?;
        Ok(())
    }

    pub(super) fn mark_corrupt(&mut self, rel_path: &str, error: &str) -> Result<()> {
        let node_id: Option<String> =
            self.corrupt_node_id.query_row(params![rel_path], |row| row.get(0)).optional()?;
        self.upsert_corrupt.execute(params![rel_path, node_id, error, Utc::now().to_rfc3339(),])?;
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct WriteMetrics {
    #[cfg(test)]
    pub(super) commits: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(super) preparations: std::cell::Cell<usize>,
}

impl WriteMetrics {
    pub(super) fn prepared(&self) {
        #[cfg(test)]
        self.preparations.set(self.preparations.get() + 1);
    }

    pub(super) fn committed(&self) {
        #[cfg(test)]
        self.commits.set(self.commits.get() + 1);
    }
}

pub(super) fn prepare_statement<'conn>(
    conn: &'conn Connection,
    sql: &str,
    metrics: &WriteMetrics,
) -> Result<Statement<'conn>> {
    let statement = conn.prepare(sql)?;
    metrics.prepared();
    Ok(statement)
}
