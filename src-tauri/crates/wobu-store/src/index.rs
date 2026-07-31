//! The SQLite index.
//!
//! This holds **no** canonical data — it is a cache of what is already in the
//! Markdown, and deleting it is always safe. It exists because reading several
//! hundred small files over SMB is slow enough to make the app feel broken; the
//! workspace renders from here and only touches the folder for changed files.
//!
//! It lives in local app data keyed by the project's ULID, never inside the
//! project folder: SQLite's POSIX advisory locking is unreliable-to-broken over
//! SMB and NFS, and WAL mode does not work there at all. The documented failure
//! mode is corruption, not an error message. See `docs/02-data-model.md`.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use wobu_core::{Asset, AssetKind, AssetLink, AssetRole, Id, LinkEdge, Node, NodeKind, NodeSummary};

use crate::atomic::Stamp;
use crate::error::Result;

/// Bumped when the table layout changes. A mismatch drops everything and
/// rebuilds from Markdown, which is why this needs no migration code.
pub const INDEX_VERSION: u32 = 4;

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
    updated_at        TEXT NOT NULL
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
"#;

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

pub struct Index {
    conn: Connection,
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

        let index = Index { conn };
        index.migrate()?;
        Ok(index)
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
                 DROP TABLE IF EXISTS corrupt;",
            )?;
            self.conn.execute_batch(SCHEMA)?;
            self.conn.execute(
                "INSERT INTO meta (key, value) VALUES ('index_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![INDEX_VERSION.to_string()],
            )?;
        }
        Ok(())
    }

    /// True when the index has never been populated, so the caller knows to do
    /// a full folder scan.
    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        Ok(n == 0)
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM nodes; DELETE FROM links; DELETE FROM asset_links;
             DELETE FROM node_fts; DELETE FROM assets; DELETE FROM corrupt;",
        )?;
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
        let displaced: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM nodes WHERE rel_path = ?1 AND id <> ?2",
                params![rel_path, node.id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(displaced) = displaced.as_deref().and_then(|s| Id::from_string(s).ok()) {
            self.remove_node(displaced)?;
        }

        self.conn.execute(
            "INSERT INTO nodes
               (id, kind, name, slug, summary, parent_id, description_state, cover_asset_id,
                rel_path, mtime_ms, size, hash, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
             ON CONFLICT(id) DO UPDATE SET
               kind=excluded.kind, name=excluded.name, slug=excluded.slug,
               summary=excluded.summary, parent_id=excluded.parent_id,
               description_state=excluded.description_state,
               cover_asset_id=excluded.cover_asset_id, rel_path=excluded.rel_path,
               mtime_ms=excluded.mtime_ms, size=excluded.size, hash=excluded.hash,
               created_at=excluded.created_at, updated_at=excluded.updated_at",
            params![
                node.id.to_string(),
                node.kind.as_str(),
                node.name,
                node.slug,
                node.summary,
                node.parent_id.map(|p| p.to_string()),
                serde_json::to_value(node.description_state)?.as_str().unwrap_or("none"),
                node.cover_asset_id.map(|a| a.to_string()),
                rel_path,
                stamp.mtime_ms,
                stamp.size as i64,
                stamp.hash,
                node.created_at.to_rfc3339(),
                node.updated_at.to_rfc3339(),
            ],
        )?;

        let id = node.id.to_string();
        self.conn.execute("DELETE FROM links WHERE from_id = ?1", params![id])?;
        for link in &node.links {
            self.conn.execute(
                "INSERT OR REPLACE INTO links (from_id, to_id, role, weight, enabled)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    link.to_id.to_string(),
                    link.role.as_str(),
                    link.weight,
                    link.enabled as i32
                ],
            )?;
        }

        // Replaced wholesale rather than merged, like the links above: the
        // frontmatter is the whole truth about this node's references, so a row
        // here that is not in the file is a row somebody deleted.
        self.conn.execute("DELETE FROM asset_links WHERE node_id = ?1", params![id])?;
        for link in &node.asset_links {
            self.conn.execute(
                "INSERT OR REPLACE INTO asset_links (node_id, asset_id, role, weight, enabled)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    link.asset_id.to_string(),
                    link.role.as_str(),
                    link.weight,
                    link.enabled as i32
                ],
            )?;
        }

        self.conn.execute("DELETE FROM node_fts WHERE id = ?1", params![id])?;
        self.conn.execute(
            "INSERT INTO node_fts (id, name, summary, notes, description)
             VALUES (?1,?2,?3,?4,?5)",
            params![id, node.name, node.summary, node.notes_raw, description_text(node)],
        )?;
        Ok(())
    }

    pub fn remove_node(&self, id: Id) -> Result<()> {
        let id = id.to_string();
        self.conn.execute("DELETE FROM nodes    WHERE id      = ?1", params![id])?;
        self.conn.execute("DELETE FROM links    WHERE from_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM links    WHERE to_id   = ?1", params![id])?;
        // The node's own links go; nothing touches `assets`, because the blob
        // outlives every node that pointed at it. Assets are content-addressed
        // and shared, so the last link going away is not a reason to delete a
        // file somebody else's node may be about to link.
        self.conn.execute("DELETE FROM asset_links WHERE node_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM node_fts WHERE id      = ?1", params![id])?;
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
    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, slug, summary, parent_id, description_state
             FROM nodes ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (id, kind, name, slug, summary, parent, state) = row?;
            let (Ok(id), Ok(kind)) = (Id::from_string(&id), kind.parse::<NodeKind>()) else {
                // A row we cannot parse means the index is out of step with the
                // build; skipping it keeps the tree usable until the rebuild.
                continue;
            };
            out.push(NodeSummary {
                id,
                kind,
                name,
                slug,
                summary,
                parent_id: parent.and_then(|p| Id::from_string(&p).ok()),
                description_state: serde_json::from_value(serde_json::Value::String(state))
                    .unwrap_or_default(),
            });
        }

        let order = |k: NodeKind| {
            wobu_core::kind_registry().iter().position(|d| d.kind == k).unwrap_or(usize::MAX)
        };
        out.sort_by_key(|n| (order(n.kind), n.name.to_lowercase()));
        Ok(out)
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
            "INSERT OR REPLACE INTO assets
               (id, hash, kind, rel_path, thumb_path, mime, width, height, bytes, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
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
            .query_row(&format!("{ASSET_COLUMNS} WHERE id = ?1"), params![id.to_string()], asset_row)
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
            .query_row("SELECT kind, parent_id FROM nodes WHERE id = ?1", params![id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
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
            .query_row("SELECT id FROM nodes WHERE kind = ?1 LIMIT 1", params![kind.as_str()], |r| {
                r.get(0)
            })
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
        let mut stmt = self.conn.prepare(
            "SELECT from_id, to_id, role, weight, enabled FROM links WHERE to_id = ?1",
        )?;
        let rows = stmt.query_map(params![id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, f32>(3)?,
                r.get::<_, i32>(4)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (from, to, role, weight, enabled) = row?;
            let (Ok(from_id), Ok(to_id)) = (Id::from_string(&from), Id::from_string(&to)) else {
                continue;
            };
            let Ok(role) = serde_json::from_value(serde_json::Value::String(role)) else {
                continue;
            };
            out.push(LinkEdge { from_id, to_id, role, weight, enabled: enabled != 0 });
        }
        Ok(out)
    }

    /// Full-text search over names, summaries, notes and descriptions.
    pub fn search(&self, query: &str) -> Result<Vec<Id>> {
        let Some(expr) = fts_match_expr(query) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT id FROM node_fts WHERE node_fts MATCH ?1 ORDER BY rank LIMIT 200",
        )?;
        let rows = stmt.query_map(params![expr], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok().and_then(|s| Id::from_string(&s).ok())).collect())
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
fn asset_link_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, String, f32, i32)> {
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
        let names: Vec<_> =
            index.list_nodes().unwrap().into_iter().map(|n| n.name).collect();
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
        node.asset_links =
            vec![AssetRef::new(asset, AssetRole::FullRef), AssetRef::new(asset, AssetRole::Palette)];
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

        assert_eq!(index.kind_and_parent(city.id).unwrap(), Some((NodeKind::Setting, Some(region.id))));
        assert_eq!(index.kind_and_parent(region.id).unwrap(), Some((NodeKind::Setting, None)));
        assert_eq!(index.children_of(region.id).unwrap(), vec![city.id]);
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
