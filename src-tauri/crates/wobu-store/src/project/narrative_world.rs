use super::Project;
use crate::{
    atomic::Stamp,
    error::Result,
    narrative::{SourceSave, layout, world},
};
use wobu_narrative::WorldDocument;

impl Project {
    pub fn world_document(&self) -> Result<Option<(WorldDocument, Stamp)>> {
        world::read(&self.root)
    }

    /// Semantic diagnostics never prevent saving a draft; format/version errors do.
    pub fn save_world(
        &mut self,
        document: &WorldDocument,
        expected: Option<&Stamp>,
    ) -> Result<SourceSave> {
        self.ensure_writable()?;
        // Only collect sidecars for quests known to exist in the exact source
        // revision being replaced. Missing, malformed or concurrently changed
        // source cannot provide deletion evidence.
        let removed = self
            .world_document()
            .ok()
            .flatten()
            .filter(|(_, stamp)| expected == Some(stamp))
            .map(|(previous, _)| {
                previous
                    .quests
                    .into_iter()
                    .filter(|quest| !document.quests.iter().any(|live| live.id == quest.id))
                    .map(|quest| quest.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let outcome = world::write(&self.root, document, expected, &self.peer)?;
        if matches!(outcome, SourceSave::Saved(_)) {
            self.index_narrative_path(world::WORLD_FILE)?;
            for quest in removed {
                // Cosmetic cleanup must never turn a committed source save into
                // an error. The layout guard refuses unsafe/symlink paths.
                let _ = layout::delete(&self.root, &layout::GraphKey::Quest { quest });
            }
        }
        Ok(outcome)
    }
}
