//! #209. Which repeated wordings somebody has already answered for.
//!
//! One policy record, not one per suppression, and keyed by the wording's
//! [`Revision`](wobu_narrative::Revision) rather than by the places it appears.
//! Keying it to the sites would mean that moving a beat, renaming a scene or
//! duplicating an asset silently revived a warning an author had already
//! answered; keying it to the digest means the answer lasts exactly as long as
//! the wording it was about, and stops the moment somebody rewrites the line.
//!
//! It is a file rather than a row in the derived index for the same reason the
//! locale policy is: the index is disposable and gets rebuilt from source, and a
//! decision a person made with a reason attached is not something to rebuild.

use super::Project;
use crate::{
    Error, NarrativeRecordDocument as Document, NarrativeRecordFile as File,
    NarrativeRecordKind as Kind, Result, SourceSave,
};
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_narrative::{WordingSuppression, review::hash};

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Malformed { path: "narrative/policies".into(), reason: e.to_string() }
}

/// The one record's identity, derived rather than generated so a project has
/// exactly one of these and two machines agree on which file it is.
fn suppressions_id() -> Id {
    let hex = hash(&"wording_suppressions_v1");
    Id::from(u128::from_str_radix(&hex[..32], 16).expect("hash hex"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    #[serde(default)]
    suppressions: Vec<WordingSuppression>,
}

impl Project {
    /// The repeated wordings this project has signed off, and the guard to hand
    /// back when changing them.
    pub fn wording_suppressions(&self) -> Result<(Vec<WordingSuppression>, String)> {
        let stored: Stored = self.production_policy(suppressions_id())?;
        Ok((stored.suppressions.clone(), hash(&stored)))
    }

    /// Replace the list, refusing a write that did not see the current one.
    ///
    /// Whole-list rather than add/remove, because the guard is what makes two
    /// writers safe on a shared folder and a per-item write would need one guard
    /// per item to say the same thing.
    pub fn save_wording_suppressions(
        &mut self,
        suppressions: Vec<WordingSuppression>,
        expected: &str,
    ) -> Result<SourceSave> {
        if let Some(blank) = suppressions.iter().find(|one| !one.is_stated()) {
            return Err(invalid(format!(
                "Give a reason for allowing revision {} to be repeated.",
                blank.revision
            )));
        }
        let (existing, guard) = self.wording_suppressions()?;
        if guard != expected {
            return Err(invalid("Suppressions changed; reload before saving."));
        }
        let mut suppressions = suppressions;
        suppressions.sort_by_key(|one| one.revision.to_string());
        suppressions.dedup_by(|a, b| a.revision == b.revision);
        let stored = Stored { suppressions };

        let file = self.narrative_record(Kind::Policy, suppressions_id())?;
        // The same check `save_locale_policy` makes: the record must still hold
        // what this read observed, so the stamp being presented is the stamp of
        // the bytes that were checked.
        if file.as_ref().is_some_and(|f| {
            f.document.payload["policy"]
                != serde_json::to_value(Stored { suppressions: existing }).unwrap()
        }) {
            return Err(invalid("Suppressions changed while capturing their stamp."));
        }
        let mut file = File {
            document: Document::new(
                Kind::Policy,
                suppressions_id(),
                "Repeated wording suppressions",
                serde_json::json!({"type":"narrative_wording_suppressions","policy":stored}),
            ),
            stamp: file.and_then(|f| f.stamp),
        };
        self.save_narrative_record(&mut file)
    }
}
