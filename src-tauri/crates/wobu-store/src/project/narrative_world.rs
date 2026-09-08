use super::Project;
use crate::{
    atomic::Stamp,
    error::Result,
    narrative::{SourceSave, world},
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
        world::write(&self.root, document, expected, &self.peer)
    }
}
