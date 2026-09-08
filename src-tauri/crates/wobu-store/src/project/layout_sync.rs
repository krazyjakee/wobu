//! Cosmetic peer records. These never enter canonical narrative replication.
use super::Project;
use crate::{Error, LayoutSave, Result, atomic, narrative::layout};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutOffer {
    pub rel: String,
    pub hash: String,
}
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutManifest {
    pub entries: Vec<LayoutOffer>,
    pub notices: Vec<String>,
}
impl Project {
    pub fn layout_manifest(&self) -> LayoutManifest {
        let mut manifest = LayoutManifest::default();
        match layout::manifest(self.root()) {
            Ok(paths) => {
                for (rel, _) in paths {
                    match layout::read_document(self.root(), &rel) {
                        Ok(Some((_, stamp))) => {
                            manifest.entries.push(LayoutOffer { rel, hash: stamp.hash })
                        }
                        Ok(None) => {}
                        Err(error) => manifest
                            .notices
                            .push(format!("Arrangement {rel} was not shared: {error}")),
                    }
                }
            }
            Err(error) => manifest.notices.push(format!("Arrangements were not shared: {error}")),
        }
        manifest.notices.truncate(20);
        manifest
    }
    pub fn layout_outgoing(&self, offer: &LayoutOffer) -> Result<Option<String>> {
        offer.validate()?;
        let Some((_, stamp)) = layout::read_document(self.root(), &offer.rel)? else {
            return Ok(None);
        };
        if stamp.hash != offer.hash {
            return Ok(None);
        }
        let current = layout::read_raw(self.root(), &offer.rel)?;
        Ok(current.filter(|(_, stamp)| stamp.hash == offer.hash).map(|(text, _)| text))
    }
    pub fn apply_layout_from_peer(&self, offer: &LayoutOffer, text: &str) -> Result<LayoutSave> {
        self.ensure_writable()?;
        offer.validate()?;
        if text.len() > layout::MAX_LAYOUT_BYTES
            || atomic::hash_bytes(text.as_bytes()) != offer.hash
        {
            return Err(Error::Malformed {
                path: offer.rel.clone().into(),
                reason: "Layout transfer exceeds its size bound or differs from its hash.".into(),
            });
        }
        let document: layout::Layout = serde_json::from_str(text)?;
        document.validate()?;
        if document.graph.rel()? != offer.rel {
            return Err(Error::Malformed {
                path: offer.rel.clone().into(),
                reason: "Layout identity differs from its path.".into(),
            });
        }
        // No source-dependent pruning in transport: arrival order must not
        // erase another machine's arrangement for a not-yet-arrived scene.
        layout::save(self.root(), &self.peer, &document, None)
    }
}
impl LayoutOffer {
    pub fn validate(&self) -> Result<()> {
        if self.rel.len() > 512 || !crate::narrative::registry::valid_hash(&self.hash) {
            return Err(Error::Malformed {
                path: self.rel.clone().into(),
                reason: "Invalid layout manifest entry.".into(),
            });
        }
        layout::graph_at(&self.rel)?;
        Ok(())
    }
}
