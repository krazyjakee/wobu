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

// One module per table group. Each contributes its own `impl Index`, so the
// type stays one type without the file having to be one file.
mod assets;
mod generations;
mod nodes;
mod peers;
mod rows;
mod schema;
mod version;

#[cfg(test)]
mod tests;

// Row decoding, prepared SQL and hashing are `index`-internal helpers every
// table module reaches for; re-exported so each one `use super::*` and stops.
pub(in crate::index) use self::rows::*;
pub(in crate::index) use self::schema::*;

pub use self::generations::{GenerationPage, GenerationPageRequest, GenerationSummary};
pub use self::version::{source_version, subject_version};

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use wobu_core::{Asset, Generation, Id, Node};

use crate::atomic::Stamp;
use crate::error::Result;

/// Bumped when the table layout changes. A mismatch drops everything and
/// rebuilds from the project folder, which is why this needs no migration code.
pub const INDEX_VERSION: u32 = 10;

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
}
