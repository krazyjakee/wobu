//! Guarded authoring records and atomic publication visibility.
use super::Project;
use crate::{
    Error, Result, SourceSave,
    atomic::{self, Stamp, WriteOutcome},
    narrative::{
        publication::{self, NarrativePublication, PublishedRecord},
        records::*,
        registry,
    },
};
use wobu_core::Id;

impl Project {
    pub fn narrative_record(
        &self,
        kind: NarrativeRecordKind,
        id: Id,
    ) -> Result<Option<NarrativeRecordFile>> {
        let rel = format!("narrative/{}/{}.json", kind.directory(), id);
        let Some((text, stamp)) = registry::read(self.root(), &rel)? else { return Ok(None) };
        let (_, _, value) = registry::parse(&rel, &text)?;
        registry::validate_references(
            self.root(),
            registry::NarrativeFileKind::Record(kind),
            &value,
        )?;
        Ok(Some(NarrativeRecordFile {
            document: serde_json::from_value(value)?,
            stamp: Some(stamp),
        }))
    }
    pub fn narrative_records(&self, kind: NarrativeRecordKind) -> Result<Vec<NarrativeRecordFile>> {
        let mut result = Vec::new();
        for (rel, _) in registry::paths(self.root())? {
            if registry::classify(&rel) == Some(registry::NarrativeFileKind::Record(kind)) {
                let Some((text, stamp)) = registry::read(self.root(), &rel)? else { continue };
                let (_, _, value) = registry::parse(&rel, &text)?;
                registry::validate_references(
                    self.root(),
                    registry::NarrativeFileKind::Record(kind),
                    &value,
                )?;
                result.push(NarrativeRecordFile {
                    document: serde_json::from_value(value)?,
                    stamp: Some(stamp),
                });
            }
        }
        Ok(result)
    }
    pub fn save_narrative_record(&mut self, file: &mut NarrativeRecordFile) -> Result<SourceSave> {
        self.ensure_writable()?;
        file.document.validate()?;
        let rel = file.document.rel();
        // An unknown/future existing record must not be silently downgraded.
        if let Some((text, _)) = registry::read(self.root(), &rel)? {
            registry::parse(&rel, &text)?;
        }
        let text = publication::canonical_record_text(&file.document)?;
        if file.document.kind == NarrativeRecordKind::Receipt {
            self.bind_receipt(&file.document)?;
        }
        let outcome = if file.document.kind.immutable() {
            SourceSave::Saved(self.write_narrative_immutable(&rel, &text)?)
        } else {
            self.write_narrative_text(&rel, &text, file.stamp.as_ref())?
        };
        if let SourceSave::Saved(stamp) = &outcome {
            file.stamp = Some(stamp.clone());
        }
        Ok(outcome)
    }
    pub(crate) fn write_narrative_text(
        &mut self,
        rel: &str,
        text: &str,
        expected: Option<&Stamp>,
    ) -> Result<SourceSave> {
        self.ensure_writable()?;
        registry::parse(rel, text)?;
        if matches!(
            registry::classify(rel),
            Some(
                registry::NarrativeFileKind::Object
                    | registry::NarrativeFileKind::ReceiptBinding
                    | registry::NarrativeFileKind::Tombstone
                    | registry::NarrativeFileKind::Restoration
                    | registry::NarrativeFileKind::Record(NarrativeRecordKind::Receipt)
            )
        ) {
            return Ok(SourceSave::Saved(self.write_narrative_immutable(rel, text)?));
        }
        let path = registry::safe_path(self.root(), rel)?;
        match atomic::guarded_write(self.root(), &path, text, expected, &self.peer)? {
            WriteOutcome::Written(stamp) => {
                self.index_narrative_path(rel)?;
                Ok(SourceSave::Saved(stamp))
            }
            WriteOutcome::Conflict { conflict_path, .. } => Ok(SourceSave::Conflict {
                conflict_path: crate::paths::to_rel_string(
                    conflict_path.strip_prefix(self.root()).unwrap_or(&conflict_path),
                ),
            }),
        }
    }
    pub(crate) fn write_narrative_immutable(&mut self, rel: &str, text: &str) -> Result<Stamp> {
        self.ensure_writable()?;
        let (_, _, value) = registry::parse(rel, text)?;
        match registry::classify(rel) {
            Some(
                registry::NarrativeFileKind::Record(NarrativeRecordKind::Receipt)
                | registry::NarrativeFileKind::Object,
            ) => {
                let document: NarrativeRecordDocument = serde_json::from_value(value)?;
                if document.kind == NarrativeRecordKind::Receipt {
                    publication::verify_receipt_binding(self.root(), &document)?;
                }
            }
            Some(registry::NarrativeFileKind::ReceiptBinding) => {
                let binding: publication::ReceiptBinding = serde_json::from_value(value)?;
                if let Some(file) =
                    self.narrative_record(NarrativeRecordKind::Receipt, binding.id)?
                    && atomic::hash_bytes(
                        publication::canonical_record_text(&file.document)?.as_bytes(),
                    ) != binding.canonical_document_hash
                {
                    return Err(Error::AlreadyExists(self.root().join(file.document.rel())));
                }
            }
            _ => {}
        }
        let path = registry::safe_path(self.root(), rel)?;
        let stamp = match atomic::write_once(self.root(), &path, text.as_bytes()) {
            Ok(stamp) => stamp,
            Err(Error::AlreadyExists(_)) => {
                let (previous, stamp) = registry::read(self.root(), rel)?
                    .ok_or_else(|| Error::NoSuchNode(rel.into()))?;
                if previous != text {
                    return Err(Error::AlreadyExists(path));
                }
                stamp
            }
            Err(error) => return Err(error),
        };
        self.index_narrative_path(rel)?;
        Ok(stamp)
    }
    /// Objects are synced first. Until the final manifest's one guarded write,
    /// readers continue seeing the previous complete revision.
    pub fn publish_narrative_records(
        &mut self,
        id: Id,
        name: &str,
        records: &[NarrativeRecordDocument],
        expected: Option<&Stamp>,
    ) -> Result<SourceSave> {
        self.ensure_writable()?;
        let mut manifest = NarrativePublication {
            schema_version: RECORD_VERSION,
            id,
            name: name.into(),
            records: Vec::new(),
        };
        let mut objects = Vec::new();
        for doc in records {
            doc.validate()?;
            let text = publication::canonical_record_text(doc)?;
            let hash = atomic::hash_bytes(text.as_bytes());
            manifest.records.push(PublishedRecord {
                id: doc.id,
                kind: doc.kind,
                hash: hash.clone(),
            });
            objects.push((format!("narrative/objects/{hash}.json"), text));
        }
        manifest.validate()?;
        let rel = manifest.rel();
        if let Some((text, _)) = registry::read(self.root(), &rel)? {
            registry::parse(&rel, &text)?;
        }
        for doc in records.iter().filter(|doc| doc.kind == NarrativeRecordKind::Receipt) {
            self.bind_receipt(doc)?;
        }
        for (rel, text) in objects {
            self.write_narrative_immutable(&rel, &text)?;
        }
        self.write_narrative_text(
            &rel,
            &(serde_json::to_string_pretty(&manifest)? + "\n"),
            expected,
        )
    }
    pub fn narrative_publication(
        &self,
        id: Id,
    ) -> Result<Option<publication::NarrativePublicationFile>> {
        let rel = format!("narrative/publications/{id}.json");
        let Some((text, stamp)) = registry::read(self.root(), &rel)? else { return Ok(None) };
        let (_, _, value) = registry::parse(&rel, &text)?;
        let manifest = serde_json::from_value(value)?;
        let records = publication::load_objects(self.root(), &manifest)?;
        Ok(Some(publication::NarrativePublicationFile { manifest, records, stamp }))
    }
    pub(super) fn bind_receipt(&mut self, doc: &NarrativeRecordDocument) -> Result<()> {
        let binding = publication::ReceiptBinding {
            schema_version: RECORD_VERSION,
            id: doc.id,
            canonical_document_hash: atomic::hash_bytes(
                publication::canonical_record_text(doc)?.as_bytes(),
            ),
        };
        // Existing standalone receipts are canonical too, including a folder
        // imported before the binding was first written.
        if let Some(file) = self.narrative_record(NarrativeRecordKind::Receipt, doc.id)?
            && file.document != *doc
        {
            return Err(Error::AlreadyExists(self.root().join(doc.rel())));
        }
        self.write_narrative_immutable(
            &binding.rel(),
            &(serde_json::to_string_pretty(&binding)? + "\n"),
        )?;
        Ok(())
    }
}
