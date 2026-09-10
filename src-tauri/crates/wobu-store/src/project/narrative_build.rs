//! Canonical build manifests reference canonical generation evidence. There is
//! no output cache to collect: deleting a local index never deletes these files.
use super::Project;
use crate::{
    Error, NarrativeRecordDocument, NarrativeRecordFile, NarrativeRecordKind, Result, SourceSave,
};
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_narrative_build::{Build, Item};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    #[serde(rename = "narrative_build")]
    Manifest { build: Build, chunks: Vec<Id> },
    #[serde(rename = "narrative_build_items")]
    Items { build_id: Id, items: Vec<Item> },
    #[serde(rename = "narrative_build_dispatch")]
    Dispatch { build_id: Id, requests: Vec<Id> },
}
fn invalid(reason: impl ToString) -> Error {
    Error::Malformed { path: "narrative/builds".into(), reason: reason.to_string() }
}
fn save(project: &mut Project, id: Id, record: &Record) -> Result<()> {
    let document = NarrativeRecordDocument::new(
        NarrativeRecordKind::Receipt,
        id,
        "Narrative build",
        serde_json::to_value(record)?,
    );
    if crate::narrative::publication::canonical_record_text(&document)?.len()
        > super::narrative_sync::MAX_NARRATIVE_FILE_BYTES
    {
        return Err(invalid("Build record exceeds the portable file limit."));
    }
    match project.save_narrative_record(&mut NarrativeRecordFile { document, stamp: None })? {
        SourceSave::Saved(_) => Ok(()),
        SourceSave::Conflict { .. } => Err(invalid("Build publication conflicted.")),
    }
}
impl Project {
    pub fn save_narrative_build(&mut self, build: &Build) -> Result<()> {
        build.validate().map_err(invalid)?;
        let mut header = build.clone();
        header.items.clear();
        let mut chunks = Vec::new();
        // Independent bounded item chunks; header is published last. Incomplete
        // capture is never visible as a resumable build.
        let limit = super::narrative_sync::MAX_NARRATIVE_FILE_BYTES;
        let mut pending = Vec::new();
        let mut bytes = 1024;
        for item in &build.items {
            let json = serde_json::to_string_pretty(item)?;
            let cost = json.len() + json.lines().count() * 8 + 4;
            if cost + 1024 > limit {
                return Err(invalid("One build item exceeds the portable file limit."));
            }
            if pending.len() == 64 || bytes + cost > limit {
                let id = wobu_core::new_id();
                save(
                    self,
                    id,
                    &Record::Items { build_id: build.id, items: std::mem::take(&mut pending) },
                )?;
                chunks.push(id);
                bytes = 1024;
            }
            bytes += cost;
            pending.push(item.clone());
        }
        if !pending.is_empty() {
            let id = wobu_core::new_id();
            save(self, id, &Record::Items { build_id: build.id, items: pending })?;
            chunks.push(id);
        }
        save(self, build.id, &Record::Manifest { build: header, chunks })
    }
    pub fn narrative_build(&self, id: Id) -> Result<Build> {
        let file = self
            .narrative_record(NarrativeRecordKind::Receipt, id)?
            .ok_or_else(|| invalid("Build manifest is missing."))?;
        let Record::Manifest { mut build, chunks } = serde_json::from_value(file.document.payload)?
        else {
            return Err(invalid("Record is not a build."));
        };
        if build.id != id || !build.items.is_empty() {
            return Err(invalid("Build identity or header is invalid."));
        }
        if chunks.len() > wobu_narrative_build::MAX_ITEMS
            || chunks.iter().collect::<std::collections::BTreeSet<_>>().len() != chunks.len()
        {
            return Err(invalid("Build chunk count or identities are invalid."));
        }
        for chunk in chunks {
            let file = self
                .narrative_record(NarrativeRecordKind::Receipt, chunk)?
                .ok_or_else(|| invalid("Build item chunk is missing."))?;
            let Record::Items { build_id, items } = serde_json::from_value(file.document.payload)?
            else {
                return Err(invalid("Invalid build chunk."));
            };
            if build_id != id {
                return Err(invalid("Build chunk identity differs."));
            }
            build.items.extend(items);
            if build.items.len() > wobu_narrative_build::MAX_ITEMS {
                return Err(invalid("Build exceeds the item limit."));
            }
        }
        build.validate().map_err(invalid)?;
        Ok(build)
    }
    pub fn narrative_build_ids(&self) -> Result<Vec<Id>> {
        Ok(self
            .narrative_records(NarrativeRecordKind::Receipt)?
            .into_iter()
            .filter(|f| {
                f.document.payload.get("type").and_then(serde_json::Value::as_str)
                    == Some("narrative_build")
            })
            .map(|f| f.document.id)
            .collect())
    }
    pub fn narrative_build_dispatched(&self, id: Id) -> Result<std::collections::BTreeSet<Id>> {
        let mut dispatched = std::collections::BTreeSet::new();
        for file in self.narrative_records(NarrativeRecordKind::Receipt)? {
            if file.document.payload.get("type").and_then(serde_json::Value::as_str)
                != Some("narrative_build_dispatch")
            {
                continue;
            }
            let Record::Dispatch { build_id, requests } =
                serde_json::from_value(file.document.payload)?
            else {
                return Err(invalid("Invalid build dispatch record."));
            };
            if build_id == id {
                dispatched.extend(requests);
            }
        }
        Ok(dispatched)
    }
    pub fn record_narrative_build_dispatch(
        &mut self,
        build_id: Id,
        requests: Vec<Id>,
    ) -> Result<()> {
        save(self, wobu_core::new_id(), &Record::Dispatch { build_id, requests })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verbose_diagnostics_split_by_bytes_and_survive_reload() {
        let temp = tempfile::tempdir().unwrap();
        let mut project = Project::create(temp.path(), "Build chunks").unwrap();
        let build = Build {
            version: wobu_narrative_build::VERSION,
            id: wobu_core::new_id(),
            scope: wobu_narrative_build::Scope::AllSelected,
            provider: "fixture".into(),
            model: "offline".into(),
            diagnostics: vec![],
            items: (0..24)
                .map(|_| Item {
                    id: wobu_core::new_id(),
                    target: wobu_narrative_context::Selection {
                        scene: wobu_narrative::SceneId::new(),
                        beat: wobu_narrative::BeatId::new(),
                        slot: wobu_narrative::DialogueSlotId::new(),
                        variant: None,
                    },
                    asset: false,
                    label: "Large diagnostic".into(),
                    action: wobu_narrative_build::Action::Blocked,
                    reasons: vec![],
                    diagnostics: vec!["x".repeat(100_000)],
                    request_id: None,
                    reusable: false,
                    candidate_variant_id: None,
                    state: Default::default(),
                })
                .collect(),
        };
        project.save_narrative_build(&build).unwrap();
        assert_eq!(project.narrative_build(build.id).unwrap().items.len(), 24);
        let records = project.narrative_records(NarrativeRecordKind::Receipt).unwrap();
        assert!(records.len() >= 3);
        for file in records {
            assert!(
                std::fs::metadata(project.root().join(file.document.rel())).unwrap().len()
                    <= super::super::narrative_sync::MAX_NARRATIVE_FILE_BYTES as u64
            );
        }
    }
}
