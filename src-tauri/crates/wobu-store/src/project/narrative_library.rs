//! Content-checked scene locators and bounded discovery over a disposable index.
use super::Project;
use crate::{Catalog, Error, Result, SceneEntry, UnreadableSource, atomic, narrative};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Inventory {
    pub catalog: Catalog,
    pub revision: String,
    pub ambiguous: BTreeSet<String>,
}
impl Project {
    pub(crate) fn scene_inventory(&self) -> Result<Inventory> {
        if !self.is_present() {
            return Err(Error::Disconnected);
        }
        let existing = self
            .index
            .scene_locators()?
            .into_iter()
            .map(|entry| (entry.rel.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let mut hashes = BTreeMap::new();
        for (rel, path) in narrative::scene_paths(self.root())? {
            let Some((text, stamp)) = atomic::read_stamped(&path)? else { continue };
            if existing
                .get(&rel)
                .is_none_or(|entry| entry.hash != stamp.hash || !entry.projection_current)
            {
                let entry = narrative::registry::entry(self.root(), &rel, &text, stamp.clone());
                self.index.upsert_narrative(&entry)?;
            }
            hashes.insert(rel, stamp.hash);
        }
        for rel in existing.keys().filter(|rel| !hashes.contains_key(*rel)) {
            self.index.remove_narrative(rel)?;
        }
        let locators = self.index.scene_locators()?;
        let mut by_id = BTreeMap::<String, Vec<String>>::new();
        for entry in &locators {
            if let Some(id) = &entry.id {
                by_id.entry(id.clone()).or_default().push(entry.rel.clone());
            }
        }
        let ambiguous = by_id
            .iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(id, _)| id.clone())
            .collect::<BTreeSet<_>>();
        let mut catalog = Catalog::default();
        for entry in locators {
            if let Some(id) = entry.id.as_ref().filter(|id| ambiguous.contains(*id)) {
                catalog.unreadable.push(UnreadableSource {rel:entry.rel,reason:format!("Scene ID {id} is ambiguous across {} files. Repair the duplicate identities before opening it.",by_id[id].len())});
                continue;
            }
            if let Some(error) = entry.error {
                catalog.unreadable.push(UnreadableSource { rel: entry.rel.clone(), reason: error });
            }
            if let Some(id) = entry.id {
                catalog.scenes.push(SceneEntry {
                    id: id.parse().map_err(|_| Error::NoSuchNode(id))?,
                    name: entry.name,
                    slug: entry
                        .rel
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .trim_end_matches(".yaml")
                        .into(),
                    rel: entry.rel,
                });
            }
        }
        let revision = blake3::hash(&serde_json::to_vec(&hashes)?).to_hex().to_string();
        Ok(Inventory { catalog, revision, ambiguous })
    }
}

mod query;
pub use query::{LibraryPage, LibraryQuery, QueryError};
