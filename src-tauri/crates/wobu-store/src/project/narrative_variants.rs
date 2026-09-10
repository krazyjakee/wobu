//! Canonical explicit analysis policies and immutable, bounded matrix reports.
use super::Project;
use crate::{
    Error, NarrativeRecordDocument as Document, NarrativeRecordFile as File,
    NarrativeRecordKind as Kind, Result, SourceSave,
};
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_narrative_variants::{self as variants, Policy, Report, Row, Target};
fn invalid(reason: impl std::fmt::Display) -> Error {
    Error::Malformed { path: "narrative/variants".into(), reason: reason.to_string() }
}
fn policy_id(target: &Target) -> Id {
    let hash = variants::hash(&("narrative_analysis_policy/1", target));
    Id::from(u128::from_str_radix(&hash[..32], 16).expect("hex digest"))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCapture {
    pub policies: Vec<Policy>,
    pub guard: String,
    #[serde(skip)]
    observations: Vec<(Id, Option<crate::atomic::Stamp>)>,
}
impl AnalysisCapture {
    pub fn binding(
        &self,
        target: &Target,
        report: Option<Id>,
    ) -> Option<wobu_narrative_generation::AnalysisBinding> {
        self.policies.iter().any(|p| &p.target == target).then(|| {
            wobu_narrative_generation::AnalysisBinding { policy_guard: self.guard.clone(), report }
        })
    }
    /// Policy files live outside `narrative_fingerprint`; always check both.
    pub fn check_current(&self, project: &Project) -> Result<()> {
        let current = project.narrative_analysis_capture()?;
        if current.guard != self.guard
            || (!self.observations.is_empty() && current.observations != self.observations)
        {
            return Err(invalid(
                "Analysis policy changed during capture. Reload before publication.",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisReport {
    pub id: Id,
    pub report: Report,
    pub source_guard: String,
    pub world_guard: String,
    pub policies: AnalysisCapture,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    #[serde(rename = "narrative_analysis_policy")]
    Policy { policy: Policy },
    #[serde(rename = "narrative_analysis_report")]
    Report { analysis: Box<AnalysisReport>, chunks: Vec<Id>, hash: String },
    #[serde(rename = "narrative_analysis_rows")]
    Rows { report: Id, rows: Vec<Row> },
}
fn save(
    project: &mut Project,
    id: Id,
    record: &Record,
    stamp: Option<crate::atomic::Stamp>,
) -> Result<()> {
    let document = Document::new(
        if matches!(record, Record::Policy { .. }) { Kind::Policy } else { Kind::Receipt },
        id,
        "Narrative variant analysis",
        serde_json::to_value(record)?,
    );
    if crate::narrative::publication::canonical_record_text(&document)?.len()
        > super::narrative_sync::MAX_NARRATIVE_FILE_BYTES
    {
        return Err(invalid(
            "Analysis record exceeds the portable file limit; lower its configured limits.",
        ));
    }
    match project.save_narrative_record(&mut File { document, stamp })? {
        SourceSave::Saved(_) => Ok(()),
        SourceSave::Conflict { .. } => {
            Err(invalid("Analysis publication conflicted. Reload before continuing."))
        }
    }
}
impl Project {
    pub fn narrative_analysis_capture(&self) -> Result<AnalysisCapture> {
        let mut policies = Vec::new();
        let mut observations = Vec::new();
        for file in self.narrative_records(Kind::Policy)? {
            if file.document.payload["type"] != "narrative_analysis_policy" {
                continue;
            }
            let Record::Policy { policy } = serde_json::from_value(file.document.payload.clone())?
            else {
                return Err(invalid("Invalid analysis policy record."));
            };
            if file.document.id != policy_id(&policy.target) {
                return Err(invalid("Analysis policy identity mismatch."));
            }
            observations.push((file.document.id, file.stamp, file.document.payload));
            policies.push(policy);
        }
        observations.sort_by_key(|o| o.0);
        policies.sort_by_key(|p| p.target.clone());
        Ok(AnalysisCapture {
            policies,
            guard: variants::hash(&observations.iter().map(|o| (&o.0, &o.2)).collect::<Vec<_>>()),
            observations: observations.into_iter().map(|(id, stamp, _)| (id, stamp)).collect(),
        })
    }
    pub fn save_narrative_analysis_policy(
        &mut self,
        policy: Policy,
        expected: &AnalysisCapture,
    ) -> Result<AnalysisCapture> {
        self.ensure_writable()?;
        expected.check_current(self)?;
        let snapshot = self.narrative_dependency_snapshot()?;
        policy
            .validate(&variants::Input {
                scenes: &snapshot.scenes,
                schema: &snapshot.schema,
                world: &snapshot.world,
            })
            .map_err(invalid)?;
        let id = policy_id(&policy.target);
        let current = self.narrative_record(Kind::Policy, id)?;
        expected.check_current(self)?;
        save(self, id, &Record::Policy { policy }, current.and_then(|f| f.stamp))?;
        self.narrative_analysis_capture()
    }
    pub fn save_narrative_analysis_report(&mut self, analysis: &AnalysisReport) -> Result<()> {
        self.ensure_writable()?;
        analysis.policies.check_current(self)?;
        if self.narrative_fingerprint()? != analysis.source_guard {
            return Err(invalid("Narrative sources changed during analysis."));
        }
        let hash = variants::hash(analysis);
        let mut header = analysis.clone();
        header.report.rows.clear();
        let mut chunks = Vec::new();
        let mut pending = Vec::new();
        let mut bytes = 1024;
        let limit = super::narrative_sync::MAX_NARRATIVE_FILE_BYTES;
        for row in &analysis.report.rows {
            let json = serde_json::to_string_pretty(row)?;
            let cost = json.len() + json.lines().count() * 8 + 4;
            if cost + 1024 > limit {
                return Err(invalid("One analysis witness exceeds the portable file limit."));
            }
            if pending.len() == 64 || bytes + cost > limit {
                let id = Id::generate();
                save(
                    self,
                    id,
                    &Record::Rows { report: analysis.id, rows: std::mem::take(&mut pending) },
                    None,
                )?;
                chunks.push(id);
                bytes = 1024;
            }
            bytes += cost;
            pending.push(row.clone());
        }
        if !pending.is_empty() {
            let id = Id::generate();
            save(self, id, &Record::Rows { report: analysis.id, rows: pending }, None)?;
            chunks.push(id);
        }
        analysis.policies.check_current(self)?;
        if self.narrative_fingerprint()? != analysis.source_guard {
            return Err(invalid("Narrative sources changed before report publication."));
        }
        save(self, analysis.id, &Record::Report { analysis: Box::new(header), chunks, hash }, None)
    }
    pub fn narrative_analysis_report(&self, id: Id) -> Result<AnalysisReport> {
        let file = self
            .narrative_record(Kind::Receipt, id)?
            .ok_or_else(|| invalid("Analysis report is missing."))?;
        let Record::Report { mut analysis, chunks, hash } =
            serde_json::from_value(file.document.payload)?
        else {
            return Err(invalid("Not an analysis report."));
        };
        if analysis.id != id
            || !analysis.report.rows.is_empty()
            || chunks.len() > 10_000
            || chunks.iter().collect::<std::collections::BTreeSet<_>>().len() != chunks.len()
        {
            return Err(invalid("Invalid analysis report identity or chunks."));
        }
        for chunk in chunks {
            let file = self
                .narrative_record(Kind::Receipt, chunk)?
                .ok_or_else(|| invalid("Analysis chunk is missing."))?;
            let Record::Rows { report, rows } = serde_json::from_value(file.document.payload)?
            else {
                return Err(invalid("Invalid analysis row chunk."));
            };
            if report != id {
                return Err(invalid("Analysis chunk belongs to another report."));
            }
            analysis.report.rows.extend(rows);
            if analysis.report.rows.len() > 10_000 {
                return Err(invalid("Analysis row count exceeds the bound."));
            }
        }
        if variants::hash(&analysis) != hash {
            return Err(invalid("Analysis report content differs from its receipt digest."));
        }
        Ok(*analysis)
    }
}
#[derive(Debug)]
pub struct Materialization {
    pub before: crate::SceneFile,
    pub after: crate::SceneFile,
    pub states: std::collections::BTreeMap<wobu_narrative::VariantId, variants::State>,
}
impl Project {
    pub fn materialize_narrative_analysis(
        &mut self,
        id: Id,
        slot: wobu_narrative::DialogueSlotId,
        rows: &[String],
    ) -> Result<Materialization> {
        self.ensure_writable()?;
        let saved = self.narrative_analysis_report(id)?;
        saved.policies.check_current(self)?;
        let snapshot = self.narrative_dependency_snapshot()?;
        if snapshot.fingerprint != saved.source_guard {
            return Err(invalid(
                "Sources changed since this matrix was planned. Replan before materializing.",
            ));
        }
        let policy = saved
            .policies
            .policies
            .iter()
            .find(|p| p.target == saved.report.target)
            .ok_or_else(|| invalid("Report policy is missing"))?;
        if variants::hash(policy) != saved.report.policy_hash
            || rows.is_empty()
            || rows.len() > policy.limits.variants
            || rows.iter().collect::<std::collections::BTreeSet<_>>().len() != rows.len()
        {
            return Err(invalid("Invalid report policy or row selection"));
        }
        let mut pending = Vec::new();
        let mut states = std::collections::BTreeMap::new();
        for id in rows {
            let row = saved
                .report
                .rows
                .iter()
                .find(|r| &r.id == id)
                .ok_or_else(|| invalid("Selected row is not in this saved matrix"))?;
            variants::verify_witness(
                variants::Input {
                    scenes: &snapshot.scenes,
                    schema: &snapshot.schema,
                    world: &snapshot.world,
                },
                policy,
                row,
            )
            .map_err(invalid)?;
            let variant = variants::candidate_id(&policy.target, slot, row);
            states.insert(variant, row.witness.as_ref().unwrap().state.clone());
            let mut text = wobu_narrative::Text::written("");
            text.lifecycle.policy = wobu_narrative::GenerationPolicy::Generated;
            pending.push(wobu_narrative::Variant {
                id: variant,
                when: Some(row.when.clone()),
                text,
            });
        }
        snapshot.check_current(self)?;
        saved.policies.check_current(self)?;
        let before = self.load_scene(policy.target.scene)?;
        let after = self.materialize_analysis_variants(&saved, slot, pending)?;
        Ok(Materialization { before, after, states })
    }
}
