//! One content-checked, prose-free graph projection for the whole project.
use super::Project;
use crate::{Result, UnreadableSource, narrative::library::Projection};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArcGraph {
    pub revision: String,
    pub scenes: Vec<Projection>,
    pub unreadable: Vec<UnreadableSource>,
}
impl Project {
    pub fn narrative_arc(&self) -> Result<ArcGraph> {
        let inventory = self.scene_inventory()?;
        let mut scenes = self
            .index
            .library_projections()?
            .into_iter()
            .filter(|scene| !inventory.ambiguous.contains(&scene.summary.id))
            .collect::<Vec<_>>();
        scenes.sort_by(|a, b| {
            a.summary.name.cmp(&b.summary.name).then(a.summary.id.cmp(&b.summary.id))
        });
        Ok(ArcGraph {
            revision: inventory.revision,
            scenes,
            unreadable: inventory.catalog.unreadable,
        })
    }
}
