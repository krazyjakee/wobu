//! Derived canonical narrative documents, including recoverable parse errors.
use super::Index;
use crate::{NarrativeIndexEntry, Result};
use rusqlite::{Connection, params};

pub(super) fn put(connection: &Connection, entry: &NarrativeIndexEntry) -> Result<()> {
    connection.execute("INSERT INTO narrative_files(rel,hash,entry) VALUES(?1,?2,?3) ON CONFLICT(rel) DO UPDATE SET hash=excluded.hash,entry=excluded.entry",params![entry.rel,entry.hash,serde_json::to_string(entry)?])?;
    Ok(())
}
impl Index {
    pub fn narrative_entries(&self) -> Result<Vec<NarrativeIndexEntry>> {
        let mut query = self.conn.prepare("SELECT entry FROM narrative_files ORDER BY rel")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
    pub(crate) fn replace_narrative(&self, entries: &[NarrativeIndexEntry]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM narrative_files", [])?;
        for entry in entries {
            put(&tx, entry)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn upsert_narrative(&self, entry: &NarrativeIndexEntry) -> Result<()> {
        put(&self.conn, entry)
    }
    pub(crate) fn remove_narrative(&self, rel: &str) -> Result<()> {
        self.conn.execute("DELETE FROM narrative_files WHERE rel=?1", [rel])?;
        Ok(())
    }
}
