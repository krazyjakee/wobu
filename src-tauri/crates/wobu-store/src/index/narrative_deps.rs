//! Rebuildable projection of immutable narrative dependency receipts.
//! Sets and reverse edges can both be discarded; canonical project receipts
//! retain the exact historical baseline and explanations after cache loss.

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
