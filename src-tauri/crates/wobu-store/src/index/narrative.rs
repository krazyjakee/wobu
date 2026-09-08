//! Derived canonical narrative documents, including recoverable parse errors.
use super::Index;
use crate::{NarrativeIndexEntry, Result};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn put(connection: &Connection, entry: &NarrativeIndexEntry) -> Result<()> {
    connection.execute("INSERT INTO narrative_files(rel,hash,entry) VALUES(?1,?2,?3) ON CONFLICT(rel) DO UPDATE SET hash=excluded.hash,entry=excluded.entry",params![entry.rel,entry.hash,serde_json::to_string(entry)?])?;
    super::narrative_library::put(connection, entry)?;
    Ok(())
}
impl Index {
    /// Exact local row bytes detect overlapping index mutations without decoding
    /// every scene into another full JSON object graph.
    pub(crate) fn narrative_signature(&self) -> Result<String> {
        let mut query =
            self.conn.prepare("SELECT rel,hash,entry FROM narrative_files ORDER BY rel")?;
        let mut rows = query.query([])?;
        let mut signature = blake3::Hasher::new();
        while let Some(row) = rows.next()? {
            let rel: String = row.get(0)?;
            let hash: String = row.get(1)?;
            let encoded: String = row.get(2)?;
            signature.update(
                crate::narrative::registry::cache::row_digest(&rel, &hash, encoded.as_bytes())
                    .as_bytes(),
            );
        }
        Ok(signature.finalize().to_hex().to_string())
    }

    pub fn narrative_entries(&self) -> Result<Vec<NarrativeIndexEntry>> {
        let mut query = self.conn.prepare("SELECT entry FROM narrative_files ORDER BY rel")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }
    pub(crate) fn replace_narrative<'a>(
        &self,
        entries: impl IntoIterator<Item = &'a NarrativeIndexEntry>,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch("DELETE FROM narrative_files; DELETE FROM narrative_scene_summary; DELETE FROM narrative_scene_text; DELETE FROM narrative_scene_variant;")?;
        for entry in entries {
            put(&tx, entry)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn upsert_narrative(&self, entry: &NarrativeIndexEntry) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        put(&tx, entry)?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn remove_narrative(&self, rel: &str) -> Result<()> {
        self.conn.execute("DELETE FROM narrative_files WHERE rel=?1", [rel])?;
        super::narrative_library::remove(&self.conn, rel)?;
        Ok(())
    }
    pub(crate) fn narrative_base(&self, peer: &str, rel: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT hash FROM narrative_sync WHERE peer=?1 AND rel=?2",
                params![peer, rel],
                |row| row.get(0),
            )
            .optional()?)
    }
    pub(crate) fn record_narrative_base(&self, peer: &str, rel: &str, hash: &str) -> Result<()> {
        self.conn.execute("INSERT INTO narrative_sync(peer,rel,hash) VALUES(?1,?2,?3) ON CONFLICT(peer,rel) DO UPDATE SET hash=excluded.hash",params![peer,rel,hash])?;
        Ok(())
    }
}
