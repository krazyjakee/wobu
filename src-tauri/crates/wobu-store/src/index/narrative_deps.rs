//! The stored reverse dependency index (#168).
//!
//! Two tables holding no canonical data, and deleting the database is always
//! safe — but they are not a *projection* of the project folder, and the
//! difference matters. `nodes` and `narrative_files` can be rebuilt from the
//! folder because they describe what the folder currently says. These describe
//! what each line was written *against*, which is a claim about the past that no
//! folder records. They are the same category as `sync_state`: safe to delete,
//! not derivable, and the cost of losing them is one rebuild and a run of
//! `Untracked` reports nobody needed.
//!
//! That is why they are not in `CLEAR_DERIVED_SQL`. A rescan replaces every row
//! that describes the folder, and quietly discarding the recorded baseline along
//! with them would turn an ordinary reconcile into "this project has never been
//! tracked". Losing them is an explicit act —
//! [`Project::forget_narrative_dependencies`](crate::Project::forget_narrative_dependencies)
//! — and recovering from it is
//! [`Project::rebuild_narrative_dependencies`](crate::Project::rebuild_narrative_dependencies),
//! which reads canonical data and nothing else. `narrative_dependencies.rs` runs
//! that round trip rather than asserting it in a comment.
//!
//! The two tables are declared with the rest of the schema in
//! the index's `schema` module, where every other table in this database is
//! declared, so that reading the layout of the index does not mean opening ten
//! files.
//!
//! `narrative_dependency_edge` is redundant in the strict sense: it is derivable
//! from the sets, and [`DependencyIndex::rebuild`] does derive it. It is stored
//! anyway because the question it answers — *which lines could this save
//! possibly have touched* — is asked on every write, and answering it by loading
//! and re-parsing every dependency set in the project would put a full scan in
//! front of every keystroke. The `key` index is what makes that a lookup.

use std::collections::BTreeSet;

use rusqlite::{Connection, params};
use wobu_narrative::VariantId;
use wobu_narrative_deps::{DependencyIndex, DependencySet, edge_keys};

use super::Index;
use crate::error::Result;

fn put(connection: &Connection, set: &DependencySet) -> Result<()> {
    let variant = set.variant().to_string();
    connection.execute(
        "INSERT INTO narrative_dependency(variant,fingerprint,dependencies) VALUES(?1,?2,?3) ON CONFLICT(variant) DO UPDATE SET fingerprint=excluded.fingerprint,dependencies=excluded.dependencies",
        params![variant, set.fingerprint(), serde_json::to_string(set)?],
    )?;
    connection.execute("DELETE FROM narrative_dependency_edge WHERE variant=?1", [&variant])?;
    for key in edge_keys(set) {
        connection.execute(
            "INSERT OR IGNORE INTO narrative_dependency_edge(key,variant) VALUES(?1,?2)",
            params![key, variant],
        )?;
    }
    Ok(())
}

impl Index {
    /// Every recorded dependency set, rebuilt into an index.
    ///
    /// The edges are re-derived rather than read back from their table, so a
    /// stored edge row can never be the thing a comparison depends on. The
    /// table is a lookup accelerator and this method proves it is nothing more.
    pub(crate) fn narrative_dependencies(&self) -> Result<DependencyIndex> {
        let mut query =
            self.conn.prepare("SELECT dependencies FROM narrative_dependency ORDER BY variant")?;
        let sets = query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str::<DependencySet>(&row?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(DependencyIndex::rebuild(sets))
    }

    /// Drop the recorded baseline.
    ///
    /// The deliberate spelling of cache loss, so that the recovery path is a
    /// thing callers and tests can exercise rather than a thing that only
    /// happens when a database file goes missing.
    pub(crate) fn forget_narrative_dependencies(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM narrative_dependency; DELETE FROM narrative_dependency_edge;",
        )?;
        Ok(())
    }

    /// Replace the whole recorded index in one transaction.
    ///
    /// Whole-index rather than incremental because a partial write is a state
    /// nothing could interpret: half the project measured against yesterday's
    /// toolchain and half against today's would report an affected set that
    /// describes neither.
    pub(crate) fn replace_narrative_dependencies(&self, index: &DependencyIndex) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch(
            "DELETE FROM narrative_dependency; DELETE FROM narrative_dependency_edge;",
        )?;
        for set in index.sets() {
            put(&tx, set)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Which recorded lines read any of these reverse-index keys.
    pub(crate) fn narrative_dependency_candidates(
        &self,
        keys: &BTreeSet<String>,
    ) -> Result<BTreeSet<VariantId>> {
        let mut query =
            self.conn.prepare("SELECT variant FROM narrative_dependency_edge WHERE key=?1")?;
        let mut found = BTreeSet::new();
        for key in keys {
            for row in query.query_map([key], |row| row.get::<_, String>(0))? {
                if let Ok(variant) = row?.parse::<VariantId>() {
                    found.insert(variant);
                }
            }
        }
        Ok(found)
    }

    /// The stored edge table as it actually is, for the test that proves it
    /// agrees with the derived one.
    pub fn narrative_dependency_edges(&self) -> Result<BTreeSet<(String, String)>> {
        let mut query = self.conn.prepare("SELECT key,variant FROM narrative_dependency_edge")?;
        let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.map(|row| Ok(row?)).collect()
    }
}
