//! Portable envelopes for authoring records. Domain payloads are validated by
//! their owning tools; this layer owns identity, versions and write policy.
use crate::{Error, Result, atomic::Stamp};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

pub const RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrativeRecordKind {
    Scenario,
    Proposal,
    Receipt,
    Policy,
    Production,
}
impl NarrativeRecordKind {
    pub const ALL: [Self; 5] =
        [Self::Scenario, Self::Proposal, Self::Receipt, Self::Policy, Self::Production];
    pub fn directory(self) -> &'static str {
        match self {
            Self::Scenario => "scenarios",
            Self::Proposal => "proposals",
            Self::Receipt => "receipts",
            Self::Policy => "policies",
            Self::Production => "production",
        }
    }
    pub fn immutable(self) -> bool {
        self == Self::Receipt
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeRecordDocument {
    pub schema_version: u32,
    pub id: Id,
    pub kind: NarrativeRecordKind,
    pub name: String,
    pub payload: serde_json::Value,
}
impl NarrativeRecordDocument {
    pub fn new(
        kind: NarrativeRecordKind,
        id: Id,
        name: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self { schema_version: RECORD_VERSION, id, kind, name: name.into(), payload }
    }
    pub fn rel(&self) -> String {
        format!("narrative/{}/{}.json", self.kind.directory(), self.id)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != RECORD_VERSION
            || self.name.trim().is_empty()
            || !self.payload.is_object()
        {
            return Err(Error::Malformed { path: self.rel().into(), reason: "Expected a supported narrative record version, nonempty name and object payload. Do not downgrade newer records.".into() });
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct NarrativeRecordFile {
    pub document: NarrativeRecordDocument,
    pub stamp: Option<Stamp>,
}
