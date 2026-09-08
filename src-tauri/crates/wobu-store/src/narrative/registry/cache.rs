//! Session-local reuse of independently parsed source, never a freshness oracle.
use super::{NarrativeFileKind, NarrativeIndexEntry, entry, paths};
use crate::{Result, atomic};
use std::{
    collections::BTreeMap,
    mem::size_of,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

// Structured allocation accounting is deliberately conservative: JSON maps
// include space for a spare B-tree node per map and tree links per entry.
const MAX_BYTES: usize = 384 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;

pub(crate) struct ParsedEntry {
    pub entry: NarrativeIndexEntry,
    digest: blake3::Hash,
}
pub(crate) struct Observation {
    pub entries: Vec<Arc<ParsedEntry>>,
    pub signature: String,
}
struct Retained {
    parsed: Arc<ParsedEntry>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
struct RetainedSources {
    entries: BTreeMap<String, Retained>,
    bytes: usize,
    clock: u64,
}
pub(crate) struct SourceCache {
    sources: Mutex<RetainedSources>,
    max_bytes: usize,
    max_entries: usize,
}
impl Default for SourceCache {
    fn default() -> Self {
        Self { sources: Mutex::default(), max_bytes: MAX_BYTES, max_entries: MAX_ENTRIES }
    }
}

/// The same framing is used over exact SQL row bytes when guarding index writes.
pub(crate) fn row_digest(rel: &str, hash: &str, encoded: &[u8]) -> blake3::Hash {
    let mut digest = blake3::Hasher::new();
    for part in [rel.as_bytes(), hash.as_bytes(), encoded] {
        digest.update(&(part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    digest.finalize()
}
impl SourceCache {
    fn retained(&self) -> MutexGuard<'_, RetainedSources> {
        self.sources.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn observe(&self, root: &Path) -> Result<Observation> {
        let mut entries = Vec::new();
        let mut signature = blake3::Hasher::new();
        for (rel, path) in paths(root)? {
            // Includes safe-path checks, actual content hashing and membership
            // on EVERY pass. Missing and malformed input is never a cache hit.
            let Some((text, stamp)) = atomic::read_stamped(&path)? else { continue };
            let cached = {
                let mut retained = self.retained();
                retained.clock += 1;
                let clock = retained.clock;
                retained.entries.get_mut(&rel).and_then(|cached| {
                    if cached.parsed.entry.hash != stamp.hash {
                        return None;
                    }
                    cached.used = clock;
                    Some(cached.parsed.clone())
                })
            };
            let parsed = match cached {
                Some(parsed) => parsed,
                None => {
                    let entry = entry(root, &rel, &text, stamp);
                    let digest = row_digest(&rel, &entry.hash, &serde_json::to_vec(&entry)?);
                    let parsed = Arc::new(ParsedEntry { entry, digest });
                    // These documents have local typed validation. Publications,
                    // receipts and their bindings always rerun external-reference
                    // verification, even if their own bytes did not change.
                    if parsed.entry.error.is_none()
                        && matches!(
                            parsed.entry.kind,
                            NarrativeFileKind::Scene
                                | NarrativeFileKind::Text
                                | NarrativeFileKind::World
                                | NarrativeFileKind::State
                        )
                    {
                        self.retain(&rel, parsed.clone());
                    }
                    parsed
                }
            };
            signature.update(parsed.digest.as_bytes());
            entries.push(parsed);
        }
        Ok(Observation { entries, signature: signature.finalize().to_hex().to_string() })
    }
    fn retain(&self, rel: &str, parsed: Arc<ParsedEntry>) {
        let entry = &parsed.entry;
        let bytes = size_of::<ParsedEntry>()
            + 12 * (size_of::<(String, Retained)>() + 2 * size_of::<usize>())
            + 2 * size_of::<usize>()
            + rel.len()
            + entry.rel.capacity()
            + entry.name.capacity()
            + entry.hash.capacity()
            + entry.id.as_ref().map_or(0, String::capacity)
            + entry.document.as_ref().map_or(0, json_allocations);
        let mut retained = self.retained();
        if let Some(old) = retained.entries.remove(rel) {
            retained.bytes -= old.bytes;
        }
        if bytes > self.max_bytes {
            return;
        }
        while retained.bytes + bytes > self.max_bytes || retained.entries.len() >= self.max_entries
        {
            let oldest = retained
                .entries
                .iter()
                .min_by_key(|(_, value)| value.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else { break };
            retained.bytes -= retained.entries.remove(&oldest).expect("retained entry").bytes;
        }
        retained.bytes += bytes;
        let used = retained.clock;
        retained.entries.insert(rel.into(), Retained { parsed, bytes, used });
    }
}
fn json_allocations(value: &serde_json::Value) -> usize {
    use serde_json::Value;
    match value {
        Value::String(text) => text.capacity(),
        Value::Array(values) => {
            values.capacity() * size_of::<Value>()
                + values.iter().map(json_allocations).sum::<usize>()
        }
        Value::Object(values) => {
            // serde_json uses BTreeMap (preserve_order is not enabled). Account
            // partially occupied node slots and child pointers conservatively.
            (values.len() * 3 + 11) * (size_of::<(String, Value)>() + 2 * size_of::<usize>())
                + values
                    .iter()
                    .map(|(key, value)| key.capacity() + json_allocations(value))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_narrative::{Scene, SceneDocument};
    #[test]
    fn retained_source_budget_accounts_for_structured_allocations_and_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        let scenes = dir.path().join("narrative/scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        for index in 0..3 {
            std::fs::write(
                scenes.join(format!("{index}.yaml")),
                SceneDocument::new(Scene::new("One")).to_yaml().unwrap(),
            )
            .unwrap();
        }
        let cache = SourceCache { sources: Mutex::default(), max_bytes: 64 * 1024, max_entries: 2 };
        let observed = cache.observe(dir.path()).unwrap();
        assert_eq!(
            observed.entries.len(),
            3,
            "eviction must not omit source from the current observation"
        );
        let retained = cache.retained();
        assert!(retained.entries.len() <= 2);
        assert!(retained.bytes <= 64 * 1024);
        assert!(retained.bytes > 0);
        drop(retained);
        let tiny = SourceCache { sources: Mutex::default(), max_bytes: 1, max_entries: 2 };
        assert_eq!(tiny.observe(dir.path()).unwrap().entries.len(), 3);
        assert!(tiny.retained().entries.is_empty());
    }
    #[test]
    fn malformed_source_is_reread_without_reusing_a_cached_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let scenes = dir.path().join("narrative/scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        let path = scenes.join("one.yaml");
        std::fs::write(&path, "scene: [broken").unwrap();
        let cache = SourceCache::default();
        let first = cache.observe(dir.path()).unwrap();
        let second = cache.observe(dir.path()).unwrap();
        assert!(first.entries[0].entry.error.is_some());
        assert!(!Arc::ptr_eq(&first.entries[0], &second.entries[0]));
        assert!(cache.retained().entries.is_empty());
        std::fs::write(&path, SceneDocument::new(Scene::new("Repaired")).to_yaml().unwrap())
            .unwrap();
        assert!(cache.observe(dir.path()).unwrap().entries[0].entry.error.is_none());
    }
}
