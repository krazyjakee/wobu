//! Immutable record objects published together through one guarded manifest.
use super::{
    records::{NarrativeRecordDocument, NarrativeRecordKind, RECORD_VERSION},
    registry::{self, NarrativeFileKind},
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedRecord {
    pub id: Id,
    pub kind: NarrativeRecordKind,
    pub hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativePublication {
    pub schema_version: u32,
    pub id: Id,
    pub name: String,
    pub records: Vec<PublishedRecord>,
}
impl NarrativePublication {
    pub fn rel(&self) -> String {
        format!("narrative/publications/{}.json", self.id)
    }
    pub fn validate(&self) -> Result<()> {
        let mut keys = std::collections::BTreeSet::new();
        if self.schema_version != RECORD_VERSION
            || self.name.trim().is_empty()
            || self.records.is_empty()
            || self
                .records
                .iter()
                .any(|r| !registry::valid_hash(&r.hash) || !keys.insert((r.kind, r.id)))
        {
            return Err(Error::Malformed {
                path: self.rel().into(),
                reason: "Invalid or unsupported narrative publication manifest.".into(),
            });
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptBinding {
    pub schema_version: u32,
    pub id: Id,
    pub canonical_document_hash: String,
}
impl ReceiptBinding {
    pub fn rel(&self) -> String {
        format!("narrative/receipt-bindings/{}.json", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeDeletion {
    pub schema_version: u32,
    pub id: Id,
    pub name: String,
    pub target: String,
    pub hash: String,
    pub original: String,
}
impl NarrativeDeletion {
    pub fn rel(&self) -> String {
        format!("narrative/deletions/{}.json", self.id)
    }
    pub fn validate(&self) -> Result<()> {
        let allowed = matches!(
            registry::classify(&self.target),
            Some(
                NarrativeFileKind::Scene
                    | NarrativeFileKind::State
                    | NarrativeFileKind::World
                    | NarrativeFileKind::Record(_)
                    | NarrativeFileKind::Publication
            )
        );
        if self.schema_version != RECORD_VERSION
            || !allowed
            || self.hash != crate::atomic::hash_bytes(self.original.as_bytes())
            || self.name.trim().is_empty()
        {
            return Err(Error::Malformed {
                path: self.rel().into(),
                reason: "Invalid or unsupported narrative deletion record.".into(),
            });
        }
        registry::parse(&self.target, &self.original)?;
        Ok(())
    }
}
pub fn parse(
    rel: &str,
    text: &str,
    kind: NarrativeFileKind,
) -> Result<(Option<String>, String, serde_json::Value)> {
    let malformed = || Error::Malformed {
        path: rel.into(),
        reason: "Narrative identity or content hash does not match its canonical path.".into(),
    };
    match kind {
        NarrativeFileKind::Restoration => {
            let restoration: NarrativeRestoration = serde_json::from_str(text)?;
            if restoration.rel() != rel || restoration.schema_version != RECORD_VERSION {
                return Err(malformed());
            }
            Ok((
                Some(restoration.id.to_string()),
                "Explicit narrative restoration".into(),
                serde_json::to_value(restoration)?,
            ))
        }
        NarrativeFileKind::ReceiptBinding => {
            let binding: ReceiptBinding = serde_json::from_str(text)?;
            if binding.rel() != rel
                || binding.schema_version != RECORD_VERSION
                || !registry::valid_hash(&binding.canonical_document_hash)
            {
                return Err(malformed());
            }
            Ok((
                Some(binding.id.to_string()),
                "Receipt identity binding".into(),
                serde_json::to_value(binding)?,
            ))
        }
        NarrativeFileKind::Object => {
            let doc: NarrativeRecordDocument = serde_json::from_str(text)?;
            doc.validate()?;
            if rel
                != format!("narrative/objects/{}.json", crate::atomic::hash_bytes(text.as_bytes()))
            {
                return Err(malformed());
            }
            Ok((Some(doc.id.to_string()), doc.name.clone(), serde_json::to_value(doc)?))
        }
        NarrativeFileKind::Publication => {
            let doc: NarrativePublication = serde_json::from_str(text)?;
            doc.validate()?;
            if doc.rel() != rel {
                return Err(malformed());
            }
            Ok((Some(doc.id.to_string()), doc.name.clone(), serde_json::to_value(doc)?))
        }
        NarrativeFileKind::Tombstone => {
            let doc: NarrativeDeletion = serde_json::from_str(text)?;
            doc.validate()?;
            if doc.rel() != rel {
                return Err(malformed());
            }
            Ok((Some(doc.id.to_string()), doc.name.clone(), serde_json::to_value(doc)?))
        }
        _ => Err(malformed()),
    }
}

pub fn load_objects(
    root: &std::path::Path,
    manifest: &NarrativePublication,
) -> Result<Vec<NarrativeRecordDocument>> {
    manifest.validate()?;
    manifest
        .records
        .iter()
        .map(|record| {
            let rel = format!("narrative/objects/{}.json", record.hash);
            let (text, _) = registry::read(root, &rel)?.ok_or_else(|| Error::Malformed {
                path: manifest.rel().into(),
                reason: format!(
                    "Publication is incomplete: missing immutable object {}.",
                    record.hash
                ),
            })?;
            let (_, _, value) = registry::parse(&rel, &text)?;
            let doc: NarrativeRecordDocument = serde_json::from_value(value)?;
            if doc.id != record.id || doc.kind != record.kind {
                return Err(Error::Malformed {
                    path: manifest.rel().into(),
                    reason: "Publication object identity mismatch.".into(),
                });
            }
            if doc.kind == NarrativeRecordKind::Receipt {
                verify_receipt_binding(root, &doc)?;
            }
            Ok(doc)
        })
        .collect()
}

pub fn canonical_record_text(document: &NarrativeRecordDocument) -> Result<String> {
    Ok(serde_json::to_string_pretty(document)? + "\n")
}
pub fn verify_receipt_binding(root: &std::path::Path, doc: &NarrativeRecordDocument) -> Result<()> {
    let rel = format!("narrative/receipt-bindings/{}.json", doc.id);
    let Some((text, _)) = registry::read(root, &rel)? else {
        return Err(Error::Malformed {
            path: rel.into(),
            reason: "Receipt publication has no immutable identity binding.".into(),
        });
    };
    let (_, _, value) = registry::parse(&rel, &text)?;
    let binding: ReceiptBinding = serde_json::from_value(value)?;
    if binding.canonical_document_hash
        != crate::atomic::hash_bytes(canonical_record_text(doc)?.as_bytes())
    {
        return Err(Error::Malformed {
            path: rel.into(),
            reason: "Receipt identity is already bound to different canonical bytes.".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct NarrativePublicationFile {
    pub manifest: NarrativePublication,
    pub records: Vec<NarrativeRecordDocument>,
    pub stamp: crate::atomic::Stamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeRestoration {
    pub schema_version: u32,
    pub id: Id,
    pub deletion: Id,
}
impl NarrativeRestoration {
    pub fn rel(&self) -> String {
        format!("narrative/restorations/{}.json", self.id)
    }
}
