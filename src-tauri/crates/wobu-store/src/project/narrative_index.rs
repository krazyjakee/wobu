//! Canonical data is always reread; local index rows are only a derived view.
use super::Project;
use crate::{NarrativeIndexEntry, Result, narrative::registry};
impl Project {
    pub(crate) fn index_narrative_path(&self, rel: &str) -> Result<()> {
        let Some((text, stamp)) = registry::read(self.root(), rel)? else {
            return self.index.remove_narrative(rel);
        };
        self.index.upsert_narrative(&registry::entry(self.root(), rel, &text, stamp))
    }
    pub fn narrative_index(&self) -> Result<Vec<NarrativeIndexEntry>> {
        self.index.narrative_entries()
    }
    pub(crate) fn reconcile_narrative(&self) -> Result<bool> {
        let entries = self.narrative_cache.observe(self.root())?;
        if self.index.narrative_signature()? == entries.signature {
            return Ok(false);
        }
        self.index.replace_narrative(entries.entries.iter().map(|entry| &entry.entry))?;
        Ok(true)
    }
}
