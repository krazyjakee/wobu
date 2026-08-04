//! The SQLite index.
//!
//! This holds **no** canonical data — it is a cache of what is already in the
//! project folder, and deleting it is always safe. It exists because reading
//! several hundred small files over SMB is slow enough to make the app feel
//! broken; the workspace renders from here and only touches the folder for
//! changed files.
//!
//! It lives in local app data keyed by the project's ULID, never inside the
//! project folder: SQLite's POSIX advisory locking is unreliable-to-broken over
//! SMB and NFS, and WAL mode does not work there at all. The documented failure
//! mode is corruption, not an error message. See `docs/02-data-model.md`.
//!
//! Two tables — `sync_state` and `sync_rejected` — are not a cache of canonical
//! project files, because no folder can tell you what a peer once held or what a
//! person once refused. They are still safe to delete, which is the rule that
//! actually matters here; losing them costs a re-compare on the next sync and a
//! run of conflict cards nobody needed, and nothing else. The argument is on
//! each table.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Statement, params};
use serde::{Deserialize, Serialize};
use wobu_core::{
    Asset, AssetKind, AssetLink, AssetRole, DescriptionState, EnhanceStamp, Generation, Id,
    LinkEdge, LinkRole, Node, NodeKind, NodeSummary, kind_def,
};

use crate::atomic::Stamp;
use crate::error::Result;

/// Bumped when the table layout changes. A mismatch drops everything and
/// rebuilds from the project folder, which is why this needs no migration code.
pub const INDEX_VERSION: u32 = 10;

const SCHEMA: &str = r#"
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

const UPSERT_NODE_SQL: &str = "INSERT INTO nodes
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

const UPSERT_ASSET_SQL: &str = "INSERT OR REPLACE INTO assets
       (id, hash, kind, rel_path, thumb_path, mime, width, height, bytes, created_at)
     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)";

const UPSERT_GENERATION_SQL: &str = "INSERT INTO generations
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

const CLEAR_DERIVED_SQL: &str = "DELETE FROM nodes; DELETE FROM links; DELETE FROM asset_links;
     DELETE FROM node_fts; DELETE FROM assets; DELETE FROM generations;
     DELETE FROM corrupt;";

#[cfg(test)]
const NODE_WRITE_STATEMENT_COUNT: usize = 10;
#[cfg(test)]
const REBUILD_STATEMENT_COUNT: usize = NODE_WRITE_STATEMENT_COUNT + 4;

/// A node file that is on disk and cannot be read.
///
/// `node_id` is `Some` when the index still remembers the entity this file
/// used to be — which is the common case, and the useful one: the navigator
/// can mark that entity broken in place instead of showing a bare path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorruptFile {
    /// Project-relative, `/`-separated. Shown to the user, so it must stay
    /// relative — an absolute path would leak the machine's layout.
    pub rel_path: String,
    pub node_id: Option<Id>,
    /// The parser's own words, kept verbatim.
    pub error: String,
    /// When it was *first* seen broken, not when it was last scanned.
    pub detected_at: String,
}

/// The indexed fields a concept tile needs. Full immutable receipts remain in
/// `generations.doc` and cross the bridge only when one tile is opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSummary {
    pub id: Id,
    pub node_id: Id,
    pub created_at: DateTime<Utc>,
    pub preset: String,
    pub view_type: Option<String>,
    pub backend: String,
    pub model: String,
    pub seed: u64,
    pub prompt_excerpt: String,
    pub first_asset_id: Option<Id>,
    pub output_count: u32,
    pub seed_source: Option<String>,
    pub used_locked_seed: Option<bool>,
    pub scene_subject_names: Vec<String>,
    /// Project-relative in the store; the command layer makes it absolute.
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationPageRequest {
    pub node_id: Option<Id>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationPage {
    pub items: Vec<GenerationSummary>,
    pub total: u64,
    pub next_offset: Option<u32>,
}

/// Which node rows have moved since a reader last asked.
///
/// The influence engine takes whole `Node`s, and materialising every one of them
/// on every call would put a query over the entire world in front of every drag
/// of a weight slider. So the writers say what they changed instead: `reconcile`
/// already knows exactly which files moved, and this carries that knowledge to
/// the one reader that can act on it ([`crate::Project::world_nodes`]).
///
/// [`Everything`](Touched::Everything) is not a shrug. It is what [`clear`] and a
/// schema rebuild mean, and it is the state a fresh index starts in — nobody has
/// read yet, so every row is news. Erring towards it is always correct and only
/// ever costs a query.
///
/// [`clear`]: Index::clear
pub(crate) enum Touched {
    Everything,
    These(BTreeSet<Id>),
}

struct NodeWriteStatements<'conn> {
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
    fn prepare(conn: &'conn Connection, metrics: &WriteMetrics) -> Result<Self> {
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

    fn remove_node(&mut self, id: &str) -> Result<()> {
        self.delete_node.execute(params![id])?;
        self.delete_links_from.execute(params![id])?;
        self.delete_links_to.execute(params![id])?;
        self.delete_asset_links.execute(params![id])?;
        self.delete_fts.execute(params![id])?;
        Ok(())
    }

    fn upsert_node(&mut self, node: &Node, rel_path: &str, stamp: &Stamp) -> Result<Option<Id>> {
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

struct RebuildStatements<'conn> {
    nodes: NodeWriteStatements<'conn>,
    upsert_asset: Statement<'conn>,
    upsert_generation: Statement<'conn>,
    corrupt_node_id: Statement<'conn>,
    upsert_corrupt: Statement<'conn>,
}

impl<'conn> RebuildStatements<'conn> {
    fn prepare(conn: &'conn Connection, metrics: &WriteMetrics) -> Result<Self> {
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

    fn upsert_asset(&mut self, asset: &Asset) -> Result<()> {
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

    fn upsert_generation(
        &mut self,
        generation: &Generation,
        rel_path: &str,
        stamp: &Stamp,
    ) -> Result<()> {
        let excerpt = generation_prompt_excerpt(&generation.compiled_prompt);
        let first_asset = generation.output_asset_ids.first().map(ToString::to_string);
        let seed_source = generation.params.get("seedSource").and_then(serde_json::Value::as_str);
        let used_locked_seed = generation
            .params
            .get("usedLockedSeed")
            .and_then(serde_json::Value::as_bool)
            .map(i32::from);
        let scene_names = generation_scene_names(generation)?;
        self.upsert_generation.execute(params![
            generation.id.to_string(),
            generation.node_id.to_string(),
            generation.created_at.to_rfc3339(),
            generation.preset,
            generation.view_type,
            generation.backend,
            generation.model,
            generation.seed as i64,
            excerpt,
            first_asset,
            generation.output_asset_ids.len() as i64,
            seed_source,
            used_locked_seed,
            scene_names,
            rel_path,
            stamp.mtime_ms,
            stamp.size as i64,
            stamp.hash,
            serde_json::to_string(generation)?,
        ])?;
        Ok(())
    }

    fn mark_corrupt(&mut self, rel_path: &str, error: &str) -> Result<()> {
        let node_id: Option<String> =
            self.corrupt_node_id.query_row(params![rel_path], |row| row.get(0)).optional()?;
        self.upsert_corrupt.execute(params![rel_path, node_id, error, Utc::now().to_rfc3339(),])?;
        Ok(())
    }
}

#[derive(Default)]
struct WriteMetrics {
    #[cfg(test)]
    commits: std::cell::Cell<usize>,
    #[cfg(test)]
    preparations: std::cell::Cell<usize>,
}

impl WriteMetrics {
    fn prepared(&self) {
        #[cfg(test)]
        self.preparations.set(self.preparations.get() + 1);
    }

    fn committed(&self) {
        #[cfg(test)]
        self.commits.set(self.commits.get() + 1);
    }
}

fn prepare_statement<'conn>(
    conn: &'conn Connection,
    sql: &str,
    metrics: &WriteMetrics,
) -> Result<Statement<'conn>> {
    let statement = conn.prepare(sql)?;
    metrics.prepared();
    Ok(statement)
}

pub struct Index {
    conn: Connection,
    /// Interior mutability because every writer here takes `&self` — the
    /// `Connection` does its own locking and the type is `!Sync`, so there is no
    /// thread for a `RefCell` to be contended from.
    touched: RefCell<Touched>,
    write_metrics: WriteMetrics,
}

impl Index {
    /// Open (or create) the index for a project, rebuilding from scratch if the
    /// schema version has moved.
    pub fn open_for(project_id: &Id) -> Result<Index> {
        let path = crate::paths::index_path(project_id);
        if let Some(parent) = path.parent() {
            crate::paths::ensure_dir(parent)?;
        }
        Index::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Result<Index> {
        let conn = Connection::open(path)?;
        Index::configure(conn)
    }

    pub fn in_memory() -> Result<Index> {
        Index::configure(Connection::open_in_memory()?)
    }

    fn configure(conn: Connection) -> Result<Index> {
        // Safe here precisely because this file is on local disk. The same
        // pragma over SMB is what corrupts databases.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // `Everything` from the start: a reader that has never looked is owed
        // every row, and starting at an empty change set would hand it a world
        // with no nodes in it.
        let index = Index {
            conn,
            touched: RefCell::new(Touched::Everything),
            write_metrics: WriteMetrics::default(),
        };
        index.migrate()?;
        Ok(index)
    }

    /// Note that one node's row has changed.
    ///
    /// A no-op once the set is [`Touched::Everything`], which is the whole point
    /// of the two-state enum: a rebuild that then upserted four thousand rows
    /// would otherwise spend the reader's saving on bookkeeping.
    fn touch(&self, id: Id) {
        if let Touched::These(ids) = &mut *self.touched.borrow_mut() {
            ids.insert(id);
        }
    }

    fn touch_everything(&self) {
        *self.touched.borrow_mut() = Touched::Everything;
    }

    #[cfg(test)]
    fn reset_write_metrics(&self) {
        self.write_metrics.commits.set(0);
        self.write_metrics.preparations.set(0);
    }

    /// What has changed since this was last called, resetting the record.
    ///
    /// Taken rather than read, so that two readers cannot both believe they have
    /// applied a change. There is exactly one — see [`Touched`].
    pub(crate) fn take_touched(&self) -> Touched {
        std::mem::replace(&mut *self.touched.borrow_mut(), Touched::These(BTreeSet::new()))
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA)?;

        let found: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'index_version'", [], |r| r.get(0))
            .optional()?;

        let current = found.as_deref().and_then(|v| v.parse::<u32>().ok());
        if current != Some(INDEX_VERSION) {
            // No migration path by design: everything here is derived, so
            // throwing it away and re-reading the folder is always correct.
            self.conn.execute_batch(
                "DROP TABLE IF EXISTS nodes;
                 DROP TABLE IF EXISTS links;
                 DROP TABLE IF EXISTS asset_links;
                 DROP TABLE IF EXISTS node_fts;
                 DROP TABLE IF EXISTS assets;
                 DROP TABLE IF EXISTS generations;
                 DROP TABLE IF EXISTS corrupt;
                 DROP TABLE IF EXISTS sync_state;
                 DROP TABLE IF EXISTS sync_rejected;",
            )?;
            self.conn.execute_batch(SCHEMA)?;
            self.conn.execute(
                "INSERT INTO meta (key, value) VALUES ('index_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![INDEX_VERSION.to_string()],
            )?;
            self.touch_everything();
        }
        Ok(())
    }

    /// True when the index has never been populated, so the caller knows to do
    /// a full folder scan.
    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    /// Empty every derived table, so the next scan can refill them from the
    /// folder's Markdown, assets and generation JSON.
    ///
    /// `sync_state` is deliberately not in the list. This is "re-read the
    /// Markdown", and re-reading the Markdown says nothing about what a peer
    /// held — the ids and the hashes a base refers to come out of the rescan
    /// unchanged, so a base that was true before it is still true after. A
    /// rebuild triggered by a corrupt index or an impatient user would
    /// otherwise silently reset agreement with every peer and turn the next
    /// sync into a wall of conflicts.
    ///
    /// `sync_rejected` is not in the list either, and the reasoning is the same
    /// sentence with a different noun: re-reading the Markdown says nothing
    /// about which versions a person looked at and refused. It is worth being
    /// explicit that the two are not symmetrical in *danger*, though, because
    /// that is the thing that would justify treating them differently. A stale
    /// base can license a fast-forward, so keeping one is the risky direction; a
    /// stale refusal can only ever suppress a card, and only for bytes byte-
    /// identical to ones a human already rejected from that same peer. Keeping
    /// it through a rebuild is therefore cheap in the one direction that matters,
    /// and dropping it would throw away a record of a human decision on the
    /// strength of an index repair the user asked for for unrelated reasons.
    pub fn clear(&self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(CLEAR_DERIVED_SQL)?;
        tx.commit()?;
        self.write_metrics.committed();
        self.touch_everything();
        Ok(())
    }

    /// Replace every derived row with one scan's already-read input.
    ///
    /// The caller deliberately finishes all folder IO before entering here.
    /// Once it does, the clear and every refill share one transaction: readers
    /// see either the previous complete index or the new complete index, and a
    /// failed row restores the previous one. Statements are prepared once for
    /// the batch rather than once per node, edge, asset, or generation.
    pub(crate) fn rebuild_from_scan(
        &self,
        assets: &[Asset],
        generations: &[(Generation, String, Stamp)],
        nodes: &[(Node, String, Stamp)],
        corrupt: &[(String, String)],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(CLEAR_DERIVED_SQL)?;
        {
            let mut statements = RebuildStatements::prepare(&tx, &self.write_metrics)?;
            for asset in assets {
                statements.upsert_asset(asset)?;
            }
            for (generation, rel_path, stamp) in generations {
                statements.upsert_generation(generation, rel_path, stamp)?;
            }
            for (node, rel_path, stamp) in nodes {
                statements.nodes.upsert_node(node, rel_path, stamp)?;
            }
            for (rel_path, error) in corrupt {
                statements.mark_corrupt(rel_path, error)?;
            }
        }
        tx.commit()?;
        self.write_metrics.committed();
        self.touch_everything();
        Ok(())
    }

    /// Return the pages freed by deletions to the filesystem.
    ///
    /// `DELETE` leaves the file the same size — SQLite keeps the pages on a
    /// free list. That is the right trade during normal use and the wrong one
    /// after a rebuild, which is the moment a user is looking at the number and
    /// wondering why clearing it changed nothing. Must run outside a
    /// transaction, so it is its own call rather than part of `clear`.
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

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

    /// Record a blob.
    ///
    /// `INSERT OR REPLACE` rather than an upsert on `id`, because there are
    /// three unique columns here and any of them can be the one that collides:
    /// a file re-imported after being renamed on disk keeps its id and changes
    /// its `rel_path`. Replacing outright is safe precisely because no other
    /// table references this one.
    pub fn upsert_asset(&self, asset: &Asset) -> Result<()> {
        self.conn.execute(
            UPSERT_ASSET_SQL,
            params![
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
            ],
        )?;
        Ok(())
    }

    /// Newest first — the library is browsed as "what did I just bring in",
    /// not alphabetically by hash, which is an order with no meaning at all.
    pub fn list_assets(&self) -> Result<Vec<Asset>> {
        let mut stmt = self.conn.prepare(&format!("{ASSET_COLUMNS} ORDER BY created_at DESC"))?;
        let rows = stmt.query_map([], asset_row)?;
        Ok(rows.filter_map(std::result::Result::ok).flatten().collect())
    }

    pub fn asset(&self, id: Id) -> Result<Option<Asset>> {
        Ok(self
            .conn
            .query_row(
                &format!("{ASSET_COLUMNS} WHERE id = ?1"),
                params![id.to_string()],
                asset_row,
            )
            .optional()?
            .flatten())
    }

    /// Every asset path the index holds, for the reconcile that compares a
    /// directory listing against it.
    pub fn asset_paths(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT rel_path FROM assets")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn remove_asset_by_rel_path(&self, rel_path: &str) -> Result<()> {
        // Only the description of the blob. Any `asset_links` row pointing at
        // it stays: the file may be back in a moment — a share reconnecting, a
        // sync client catching up — and dropping the links would quietly strip
        // the roles out of somebody's node while their folder was mid-sync.
        self.conn.execute("DELETE FROM assets WHERE rel_path = ?1", params![rel_path])?;
        Ok(())
    }

    /* ── generations ────────────────────────────────────────────────────
     *
     * Canonically one immutable JSON document per row. The columns here are a
     * read model for the Concepts grid; deleting every one and scanning the
     * month shards produces exactly the same rows again.
     */

    pub fn upsert_generation(
        &self,
        generation: &Generation,
        rel_path: &str,
        stamp: &Stamp,
    ) -> Result<()> {
        let excerpt = generation_prompt_excerpt(&generation.compiled_prompt);
        let first_asset = generation.output_asset_ids.first().map(ToString::to_string);
        let seed_source = generation.params.get("seedSource").and_then(serde_json::Value::as_str);
        let used_locked_seed = generation
            .params
            .get("usedLockedSeed")
            .and_then(serde_json::Value::as_bool)
            .map(i32::from);
        let scene_names = generation_scene_names(generation)?;
        self.conn.execute(
            UPSERT_GENERATION_SQL,
            params![
                generation.id.to_string(),
                generation.node_id.to_string(),
                generation.created_at.to_rfc3339(),
                generation.preset,
                generation.view_type,
                generation.backend,
                generation.model,
                generation.seed as i64,
                excerpt,
                first_asset,
                generation.output_asset_ids.len() as i64,
                seed_source,
                used_locked_seed,
                scene_names,
                rel_path,
                stamp.mtime_ms,
                stamp.size as i64,
                stamp.hash,
                serde_json::to_string(generation)?,
            ],
        )?;
        Ok(())
    }

    /// One generation from local disk, without opening its canonical JSON.
    pub fn generation(&self, id: Id) -> Result<Option<Generation>> {
        let doc: Option<String> = self
            .conn
            .query_row(
                "SELECT doc FROM generations WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(doc.and_then(|doc| serde_json::from_str(&doc).ok()))
    }

    /// Whether any receipt still on the visible ledger claims this asset as an
    /// output.
    ///
    /// Assets are content-addressed, so two runs that produced identical bytes
    /// share one id and one file. Deleting one of those concepts must not take
    /// the picture out from under the other, which is why this asks the whole
    /// table rather than trusting the receipt being deleted. `first_asset_id`
    /// answers the common single-output case from a column; the `LIKE` widens
    /// it to multi-output receipts and the parse confirms the match, so a hit
    /// on the same id inside a prompt or an influence snapshot cannot pass for
    /// an output.
    pub fn generation_outputs_contain(&self, asset_id: Id) -> Result<bool> {
        let id = asset_id.to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT doc FROM generations WHERE first_asset_id = ?1 OR doc LIKE ?2")?;
        let candidates =
            collect_json_documents(stmt.query_map(params![id, format!("%{id}%")], document_row)?)?;
        Ok(candidates
            .iter()
            .any(|generation: &Generation| generation.output_asset_ids.contains(&asset_id)))
    }

    /// Full node receipts for the mesh adapter's immutable turnaround joins.
    pub fn generation_documents_for_node(&self, node_id: Id) -> Result<Vec<Generation>> {
        let mut stmt = self.conn.prepare(
            "SELECT doc FROM generations
             WHERE node_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        collect_json_documents(stmt.query_map(params![node_id.to_string()], document_row)?)
    }

    /// A bounded page of lightweight receipt rows, newest first.
    ///
    /// Every predicate is answered by an indexed/scalar column. The large JSON
    /// `doc` is deliberately absent from this query and is read only by
    /// [`generation`](Self::generation) after a person opens one receipt.
    pub fn generation_page(&self, request: &GenerationPageRequest) -> Result<GenerationPage> {
        let node_id = request.node_id.map(|id| id.to_string());
        let limit = request.limit.clamp(1, 100);
        let predicate = "(?1 IS NULL OR g.node_id = ?1)";
        let sql = format!(
            "SELECT g.id, g.node_id, g.created_at, g.preset, g.view_type,
                    g.backend, g.model, g.seed, g.prompt_excerpt,
                    g.first_asset_id, g.output_count, g.seed_source,
                    g.used_locked_seed, g.scene_subject_names, a.thumb_path
             FROM generations g
             LEFT JOIN assets a ON a.id = g.first_asset_id
             WHERE {predicate}
             ORDER BY g.created_at DESC, g.id DESC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows =
            stmt.query_map(params![node_id, limit, request.offset], generation_summary_row)?;
        let mut items = Vec::new();
        for row in rows {
            if let Some(summary) = row? {
                items.push(summary);
            }
        }

        let count_sql = format!("SELECT COUNT(*) FROM generations g WHERE {predicate}");
        let total =
            self.conn.query_row(&count_sql, params![node_id], |row| row.get::<_, i64>(0))? as u64;
        let consumed = request.offset.saturating_add(items.len() as u32);
        Ok(GenerationPage {
            items,
            total,
            next_offset: (u64::from(consumed) < total).then_some(consumed),
        })
    }

    /// Every generation path held by the disposable index.
    pub fn generation_paths(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT rel_path FROM generations")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn remove_generation_by_rel_path(&self, rel_path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM generations WHERE rel_path = ?1", params![rel_path])?;
        Ok(())
    }

    /* ── asset links ─────────────────────────────────────────────────────
     *
     * Canonically frontmatter, cached here. `upsert_node` is the only writer,
     * which is what makes these rows rebuildable: every path that reads a node
     * file — first scan, reconcile, save — already goes through it.
     */

    /// Every asset attached to a node, strongest first.
    ///
    /// Ordered by weight because every caller is either drawing a strip of
    /// references or filling a budget, and both want the ones that matter most
    /// first. Ties break on role so the order is stable between calls rather
    /// than left to SQLite's row order.
    pub fn asset_links_of(&self, node_id: Id) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE node_id = ?1 ORDER BY weight DESC, role, asset_id",
        )?;
        collect_asset_links(stmt.query_map(params![node_id.to_string()], asset_link_row)?)
    }

    /// The same, narrowed to one role — what the per-role image budget asks.
    ///
    /// Filtered in SQL rather than by the caller so that the covering index on
    /// `(node_id, role)` is what does the work; a budget that pulled every
    /// reference on a node and then discarded six sevenths of them would make
    /// the index pointless.
    pub fn asset_links_in_role(&self, node_id: Id, role: AssetRole) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE node_id = ?1 AND role = ?2 ORDER BY weight DESC, asset_id",
        )?;
        collect_asset_links(
            stmt.query_map(params![node_id.to_string(), role.as_str()], asset_link_row)?,
        )
    }

    /// Every node using this asset — "3 characters reference this".
    ///
    /// Also the honest answer to "is anything still using this file", which is
    /// a question the library will want to ask. It is deliberately *not* wired
    /// to deletion: an asset with no links is still a file the user imported.
    pub fn asset_backlinks(&self, asset_id: Id) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE asset_id = ?1 ORDER BY node_id, role",
        )?;
        collect_asset_links(stmt.query_map(params![asset_id.to_string()], asset_link_row)?)
    }

    /// A node's cover image, without opening its file.
    ///
    /// A column rather than a read because the surfaces that want it — the
    /// Launcher's project cards, any grid of tiles — want one per node at once,
    /// and the whole point of the index is that a card does not cost a file
    /// read over SMB.
    pub fn cover_asset_of(&self, node_id: Id) -> Result<Option<Id>> {
        let raw: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT cover_asset_id FROM nodes WHERE id = ?1",
                params![node_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw.flatten().and_then(|s| Id::from_string(&s).ok()))
    }

    /* ── corrupt files ───────────────────────────────────────────────────
     *
     * A sync client copying a half-written file is the expected cause, and a
     * truncated YAML frontmatter block the expected shape. Three rules follow,
     * and all three are about *not* acting:
     *
     * - the file is never written over, so the user's bytes survive;
     * - its node row is never removed, because a live row next to a broken
     *   file is how the user finds their data again;
     * - the parse error is kept verbatim, because "expected a mapping at line
     *   4" is the only thing that tells them what to fix.
     */

    /// Record a file that is present but unparseable. Idempotent — reconcile
    /// re-runs on every folder event and must not accumulate rows.
    pub fn mark_corrupt(&self, rel_path: &str, error: &str) -> Result<()> {
        // The node id, where there is one, is what lets the navigator draw the
        // broken state *on* the entity the user knows, rather than as an
        // orphaned path they have to recognise.
        let node_id: Option<String> = self
            .conn
            .query_row("SELECT id FROM nodes WHERE rel_path = ?1", params![rel_path], |r| r.get(0))
            .optional()?;

        // `detected_at` is only written on insert, so a file that stays broken
        // keeps the time it first broke rather than the time of the last scan.
        self.conn.execute(
            "INSERT INTO corrupt (rel_path, node_id, error, detected_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(rel_path) DO UPDATE SET node_id = ?2, error = ?3",
            params![rel_path, node_id, error, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// The file parsed, or went away. Either way it is no longer broken.
    pub fn clear_corrupt(&self, rel_path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM corrupt WHERE rel_path = ?1", params![rel_path])?;
        Ok(())
    }

    pub fn corrupt_files(&self) -> Result<Vec<CorruptFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT rel_path, node_id, error, detected_at FROM corrupt ORDER BY rel_path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (rel_path, node_id, error, detected_at) = row?;
            out.push(CorruptFile {
                rel_path,
                node_id: node_id.as_deref().and_then(|s| Id::from_string(s).ok()),
                error,
                detected_at,
            });
        }
        Ok(out)
    }

    /// Every path currently recorded as corrupt, for the sweep that drops the
    /// ones whose file has since been deleted.
    pub fn corrupt_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT rel_path FROM corrupt")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

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

/// Turn whatever the user typed into an FTS5 `MATCH` expression, or `None` when
/// there is nothing worth searching for.
///
/// Two things are going on here, and they pull in opposite directions.
///
/// **Every term is quoted**, so that FTS5 operators the user did not mean — a
/// stray `-`, `*`, `"`, or the word `AND` — cannot become a syntax error. The
/// palette searches on every keystroke, so a query is malformed far more often
/// than it is complete.
///
/// **But the terms are joined with `AND` rather than quoted as one phrase.**
/// Quoting the whole query makes it an adjacency test, so "scarred enforcer"
/// misses notes reading "scarred ex-guild enforcer" — which is exactly the
/// query someone types when half-remembering a phrase, and exactly the case
/// searching notes exists to serve. Each term keeps its `*` so prefixes still
/// match while the word is still being typed.
///
/// Terms with no alphanumeric character are dropped rather than escaped: `"-"`
/// tokenizes to nothing, and a query made only of those should find nothing
/// rather than error.
fn fts_match_expr(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" AND ")) }
}

/// Raw navigator columns, decoded once and converted separately so a row from
/// a newer build can be skipped without turning a disposable-index mismatch
/// into a query failure.
struct NodeSummaryRow {
    id: String,
    kind: String,
    name: String,
    slug: String,
    summary: String,
    parent: Option<String>,
    state: String,
}

impl NodeSummaryRow {
    fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            slug: row.get(3)?,
            summary: row.get(4)?,
            parent: row.get(5)?,
            state: row.get(6)?,
        })
    }

    fn into_summary(self, stale: &BTreeSet<Id>) -> Option<NodeSummary> {
        let id = Id::from_string(&self.id).ok()?;
        let kind = self.kind.parse::<NodeKind>().ok()?;
        let description_state = if stale.contains(&id) {
            DescriptionState::Stale
        } else {
            serde_json::from_value(serde_json::Value::String(self.state)).unwrap_or_default()
        };
        Some(NodeSummary {
            id,
            kind,
            name: self.name,
            slug: self.slug,
            summary: self.summary,
            parent_id: self.parent.and_then(|parent| Id::from_string(&parent).ok()),
            description_state,
        })
    }
}

struct StalenessRow {
    id: String,
    state: String,
    subject: String,
    source: String,
    stamp: String,
}

impl StalenessRow {
    fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            state: row.get(1)?,
            subject: row.get(2)?,
            source: row.get(3)?,
            stamp: row.get(4)?,
        })
    }
}

fn document_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    row.get(0)
}

fn generation_prompt_excerpt(prompt: &str) -> String {
    const LIMIT: usize = 240;
    let mut excerpt: String = prompt.chars().take(LIMIT).collect();
    if prompt.chars().count() > LIMIT {
        excerpt.push('…');
    }
    excerpt
}

fn generation_scene_names(generation: &Generation) -> Result<String> {
    let names = generation
        .params
        .get("sceneComposition")
        .and_then(|scene| scene.get("subjectNames"))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(serde_json::to_string(&names)?)
}

fn generation_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<GenerationSummary>> {
    let id = row.get::<_, String>(0)?;
    let node_id = row.get::<_, String>(1)?;
    let created_at = row.get::<_, String>(2)?;
    let (Ok(id), Ok(node_id), Ok(created_at)) = (
        Id::from_string(&id),
        Id::from_string(&node_id),
        DateTime::parse_from_rfc3339(&created_at),
    ) else {
        return Ok(None);
    };
    let first_asset_id =
        row.get::<_, Option<String>>(9)?.and_then(|value| Id::from_string(&value).ok());
    let scene_subject_names =
        serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_else(|_| Vec::new());
    Ok(Some(GenerationSummary {
        id,
        node_id,
        created_at: created_at.with_timezone(&Utc),
        preset: row.get(3)?,
        view_type: row.get(4)?,
        backend: row.get(5)?,
        model: row.get(6)?,
        seed: row.get::<_, i64>(7)? as u64,
        prompt_excerpt: row.get(8)?,
        first_asset_id,
        output_count: row.get::<_, i64>(10)? as u32,
        seed_source: row.get(11)?,
        used_locked_seed: row.get::<_, Option<i32>>(12)?.map(|value| value != 0),
        scene_subject_names,
        thumbnail_path: row.get(14)?,
    }))
}

fn collect_json_documents<T, I>(rows: I) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
    I: Iterator<Item = rusqlite::Result<String>>,
{
    let mut out = Vec::new();
    for row in rows {
        if let Ok(document) = serde_json::from_str::<T>(&row?) {
            out.push(document);
        }
    }
    Ok(out)
}

struct LinkRow {
    from: String,
    to: String,
    role: String,
    weight: f32,
    enabled: i32,
}

impl LinkRow {
    fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            from: row.get(0)?,
            to: row.get(1)?,
            role: row.get(2)?,
            weight: row.get(3)?,
            enabled: row.get(4)?,
        })
    }

    fn into_edge(self) -> Option<LinkEdge> {
        let from_id = Id::from_string(&self.from).ok()?;
        let to_id = Id::from_string(&self.to).ok()?;
        let role = serde_json::from_value(serde_json::Value::String(self.role)).ok()?;
        Some(LinkEdge { from_id, to_id, role, weight: self.weight, enabled: self.enabled != 0 })
    }
}

fn collect_links<I>(rows: I) -> Result<Vec<LinkEdge>>
where
    I: Iterator<Item = rusqlite::Result<LinkRow>>,
{
    let mut out = Vec::new();
    for row in rows {
        if let Some(edge) = row?.into_edge() {
            out.push(edge);
        }
    }
    Ok(out)
}

const ASSET_COLUMNS: &str = "SELECT id, hash, kind, rel_path, thumb_path, mime, width, height,
                                    bytes, created_at
                             FROM assets";

/// One row as an [`Asset`], or `None` for a row this build cannot make sense
/// of. The same forgiveness [`Index::list_nodes`] shows, for the same reason: a
/// single unreadable row must not take the whole library off the screen.
fn asset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Asset>> {
    let (id, kind, created) =
        (r.get::<_, String>(0)?, r.get::<_, String>(2)?, r.get::<_, String>(9)?);
    let (Ok(id), Ok(kind), Ok(created_at)) = (
        Id::from_string(&id),
        serde_json::from_value::<AssetKind>(serde_json::Value::String(kind)),
        DateTime::parse_from_rfc3339(&created),
    ) else {
        return Ok(None);
    };
    Ok(Some(Asset {
        id,
        hash: r.get(1)?,
        kind,
        rel_path: r.get(3)?,
        thumb_path: r.get(4)?,
        mime: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        bytes: r.get::<_, i64>(8)? as u64,
        created_at: created_at.with_timezone(&Utc),
    }))
}

/// One `asset_links` row as the tuple the collector parses.
fn asset_link_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, String, f32, i32)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

/// Rows to [`AssetLink`]s, skipping any this build cannot parse.
///
/// The same forgiveness the node and asset readers show, and the same reason: a
/// role written by a newer Wobu must cost the user that one reference, not the
/// whole strip.
fn collect_asset_links<I>(rows: I) -> Result<Vec<AssetLink>>
where
    I: Iterator<Item = rusqlite::Result<(String, String, String, f32, i32)>>,
{
    let mut out = Vec::new();
    for row in rows {
        let (node, asset, role, weight, enabled) = row?;
        let (Ok(node_id), Ok(asset_id)) = (Id::from_string(&node), Id::from_string(&asset)) else {
            continue;
        };
        let Ok(role) = serde_json::from_value::<AssetRole>(serde_json::Value::String(role)) else {
            continue;
        };
        out.push(AssetLink { asset_id, node_id, role, weight, enabled: enabled != 0 });
    }
    Ok(out)
}

/* ── versions ─────────────────────────────────────────────────────────────
 *
 * A "version" is a short content hash of the bytes an enhance actually reads.
 * `wobu_core::EnhanceStamp` argues why it is a hash and not a timestamp; these
 * two decide *which* bytes, and what is left out matters as much as what is in.
 */

/// The version of a node **as an influence source** — what a description
/// downstream of it was enhanced from.
///
/// Two things go in. **The description**, because the enhance context is built
/// from the resolved stack's descriptions rather than their raw notes
/// (`docs/04-influence-engine.md`); rewriting a species' notes cannot have
/// changed a word of a character enhanced under it, so it must not invalidate
/// one. **The edges** — `parent_id` and the enabled links — because they decide
/// which sources a stack reaches at all: a culture that gains a `located_in`
/// puts a whole place chain in front of every character in it, and that is a
/// change to their input even though no description moved.
///
/// The edges are also what make a stamp self-sufficient. A source can only
/// enter a stack through an edge on a node already in that stack, so a stamp
/// that watches its recorded sources' edges cannot miss one appearing — and
/// staleness needs no walk to answer.
///
/// Left out: `name`, `summary`, link `weight`, `updated_at`. Weight is a
/// compile-time knob that never reaches the enhance context, and the rest are
/// labels. Marking a hundred descriptions stale because somebody fixed a typo
/// in a species name is the noise that teaches people to ignore the dot.
pub fn source_version(node: &Node) -> String {
    let description = node
        .description
        .as_ref()
        .map(|d| serde_json::to_string(d).unwrap_or_default())
        .unwrap_or_default();
    version(&[&description, &edges(node)])
}

/// The version of a node **as an enhance subject** — its own half of the
/// context.
///
/// Its notes and attributes, which are what the model is asked to elaborate,
/// plus its edges, which decide its stack. Its own description is deliberately
/// absent: that is the *output*, and including it would make a description
/// stale the instant it was written, and make every hand-edit register as
/// staleness rather than as the resolution it is.
pub fn subject_version(node: &Node) -> String {
    let attributes = serde_json::to_string(&node.attributes).unwrap_or_default();
    version(&[&node.notes_raw, &attributes, &edges(node)])
}

/// A node's edges, canonically. Sorted rather than left in file order: shuffling
/// the `links:` block in Obsidian changes nothing the enhance context would see,
/// and hashing file order would report that reshuffle as a change to every
/// description downstream.
fn edges(node: &Node) -> String {
    let mut out: Vec<String> = node
        .links
        .iter()
        .filter(|l| l.enabled)
        .map(|l| format!("{}:{}", l.to_id, l.role.as_str()))
        .collect();
    out.sort();
    if let Some(parent) = node.parent_id {
        out.insert(0, format!("{parent}:parent"));
    }
    out.join(",")
}

/// BLAKE3 over length-prefixed parts.
///
/// Length-prefixed rather than delimited because a delimiter can appear inside
/// somebody's notes, and `["a", "bc"]` hashing the same as `["ab", "c"]` is a
/// change this would then never see.
///
/// Truncated to 64 bits, which goes into frontmatter a person reads in
/// Obsidian. This answers "did these bytes change", not "could an adversary
/// forge these bytes"; a full hash would be four lines of noise per source in
/// every enhanced file, and people abandon formats that look like that.
fn version(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().to_hex()[..16].to_string()
}

fn description_text(node: &Node) -> String {
    let Some(description) = &node.description else { return String::new() };
    let mut out = String::new();
    for value in description.sections.values() {
        match value {
            wobu_core::SectionValue::Text(t) => {
                out.push_str(t);
                out.push('\n');
            }
            wobu_core::SectionValue::List(items) => {
                for item in items {
                    out.push_str(item);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::asset::AssetRef;
    use wobu_core::{Link, LinkRole};

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
        let four_thousand =
            (index.write_metrics.commits.get(), index.write_metrics.preparations.get());

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
        node.asset_links = vec![
            AssetRef::new(asset, AssetRole::FullRef),
            AssetRef::new(asset, AssetRole::Palette),
        ];
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

        assert_eq!(
            index.kind_and_parent(city.id).unwrap(),
            Some((NodeKind::Setting, Some(region.id)))
        );
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
}
