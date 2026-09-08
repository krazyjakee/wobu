use super::*;
use wobu_narrative::GenerationPolicy;
use wobu_narrative_generation::{AttemptStatus, PublicationChecks, Receipt, VERSION};
use wobu_store::{NarrativeRecordDocument, NarrativeRecordFile, NarrativeRecordKind, SourceSave};

pub fn save_receipt(
    project: &mut Project,
    id: Id,
    name: &str,
    receipt: &Receipt,
) -> CommandResult<()> {
    let mut file = NarrativeRecordFile {
        document: NarrativeRecordDocument::new(
            NarrativeRecordKind::Receipt,
            id,
            name.to_owned(),
            serde_json::to_value(receipt).map_err(invalid)?,
        ),
        stamp: None,
    };
    match project.save_narrative_record(&mut file)? {
        SourceSave::Saved(_) => Ok(()),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

pub fn request(project: &Project, id: Id) -> CommandResult<FrozenRequest> {
    let file = project
        .narrative_record(NarrativeRecordKind::Receipt, id)?
        .ok_or_else(|| invalid("Generation request no longer exists."))?;
    let Receipt::NarrativeGenerationRequest { request } =
        serde_json::from_value(file.document.payload).map_err(invalid)?
    else {
        return Err(invalid("Record is not a narrative generation request."));
    };
    request.validate().map_err(invalid)?;
    if request.request_id != id {
        return Err(invalid("Frozen request identity does not match its receipt."));
    }
    Ok(*request)
}

/// Parse the canonical receipt set once and group attempts before serving history.
pub struct RecordSet {
    pub requests: BTreeMap<Id, FrozenRequest>,
    by_request: BTreeMap<Id, Vec<(Id, Receipt)>>,
}
impl RecordSet {
    pub fn load(project: &Project) -> CommandResult<Self> {
        let mut result = Self { requests: BTreeMap::new(), by_request: BTreeMap::new() };
        for document in wobu_store::project::narrative_generation::receipts(project)? {
            let tag = document.payload.get("type").and_then(serde_json::Value::as_str);
            if !matches!(tag, Some("narrative_generation_request" | "narrative_generation_attempt"))
            {
                continue;
            }
            let receipt: Receipt = serde_json::from_value(document.payload).map_err(invalid)?;
            match receipt {
                Receipt::NarrativeGenerationRequest { request } => {
                    request.validate().map_err(invalid)?;
                    if request.request_id != document.id {
                        return Err(invalid("Frozen request identity does not match its receipt."));
                    }
                    result.requests.insert(request.request_id, *request);
                }
                Receipt::NarrativeGenerationAttempt { version, request_id, attempt, .. } => {
                    if version != VERSION || attempt == 0 {
                        return Err(invalid(
                            "Invalid or unsupported generation attempt version/counter.",
                        ));
                    }
                    result.by_request.entry(request_id).or_default().push((document.id, receipt));
                }
            }
        }
        for entries in result.by_request.values_mut() {
            entries.sort_by_key(|(_, r)| match r {
                Receipt::NarrativeGenerationAttempt { attempt, .. } => *attempt,
                _ => 0,
            });
        }
        Ok(result)
    }
    pub fn attempts(&self, request: &FrozenRequest) -> CommandResult<Vec<(Id, Receipt)>> {
        let entries = self.by_request.get(&request.request_id).cloned().unwrap_or_default();
        let expected = request.hash();
        if entries.iter().any(|(_, r)| match r {
            Receipt::NarrativeGenerationAttempt { request_hash, .. } => *request_hash != expected,
            _ => true,
        }) {
            return Err(invalid("Generation receipt does not match its frozen request."));
        }
        Ok(entries)
    }
}
pub fn attempts(project: &Project, request: &FrozenRequest) -> CommandResult<Vec<(Id, Receipt)>> {
    RecordSet::load(project)?.attempts(request)
}

pub fn checks(project: &Project, request: &FrozenRequest) -> CommandResult<PublicationChecks> {
    let file = project.load_scene(request.target.scene)?;
    let slot = file
        .scene
        .beats
        .iter()
        .find(|b| b.id == request.target.beat)
        .and_then(|b| b.dialogue.iter().find(|s| s.id == request.target.slot));
    let variant =
        slot.and_then(|s| s.variants.iter().find(|v| Some(v.id) == request.target.variant));
    let text_unchanged = if request.target.variant.is_none() {
        slot.is_some_and(|s| s.variants.is_empty() && s.speaker == request.speaker)
    } else {
        variant.is_some_and(|v| {
            Some(&v.text.revision) == request.expected_text_revision.as_ref()
                && v.text.revision_matches()
        })
    };
    let policy = variant.map(|v| v.text.lifecycle.policy);
    let current =
        super::super::narrative_context::capture(project, request.context.options.clone(), || {});
    Ok(PublicationChecks {
        scene_unchanged: file.stamp.as_ref().is_some_and(|s| s.hash == request.expected_scene_hash),
        text_unchanged,
        policy_unchanged: policy == request.expected_policy
            && slot.is_some_and(|s| s.policy == request.expected_slot_policy),
        context_unchanged: current.is_ok_and(|c| c.ready && c.hash == request.context.hash),
        locked_now: policy == Some(GenerationPolicy::Locked)
            || slot.is_some_and(|s| s.policy == GenerationPolicy::Locked),
    })
}

/// A successful receipt is written first. Publication can be repaired without another paid call.
pub fn publish(
    project: &mut Project,
    request: &FrozenRequest,
    receipt_id: Id,
    receipt: &Receipt,
) -> CommandResult<()> {
    let Receipt::NarrativeGenerationAttempt {
        status: AttemptStatus::Succeeded,
        raw_accepted_output: Some(raw),
        candidate: Some(candidate),
        ..
    } = receipt
    else {
        return Err(invalid("Only a validated successful attempt can publish a proposal."));
    };
    if request.validate_output(raw).map_err(invalid)? != *candidate {
        return Err(invalid("Recorded candidate differs from the validated provider output."));
    }
    // Validate the entire owning-domain pair before claiming an existing manifest is complete.
    if published(project, request, receipt_id, receipt)?.is_some() {
        return Ok(());
    }
    let publication_checks = checks(project, request).unwrap_or(PublicationChecks {
        scene_unchanged: false,
        text_unchanged: false,
        policy_unchanged: false,
        context_unchanged: false,
        locked_now: false,
    });
    let proposal = wobu_store::project::narrative_generation::proposal(
        request,
        receipt_id,
        candidate.clone(),
        publication_checks,
    );
    let records = [
        NarrativeRecordDocument::new(
            NarrativeRecordKind::Receipt,
            receipt_id,
            "Narrative generation attempt",
            serde_json::to_value(receipt).map_err(invalid)?,
        ),
        NarrativeRecordDocument::new(
            NarrativeRecordKind::Proposal,
            receipt_id,
            "Narrative dialogue proposal",
            serde_json::to_value(proposal).map_err(invalid)?,
        ),
    ];
    match project.publish_narrative_records(
        receipt_id,
        "Narrative generation result",
        &records,
        None,
    )? {
        SourceSave::Saved(_) => {
            let _ = project.apply_generated_proposal(receipt_id);
            Ok(())
        }
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

pub fn published(
    project: &Project,
    request: &FrozenRequest,
    receipt_id: Id,
    receipt: &Receipt,
) -> CommandResult<Option<PublicationChecks>> {
    Ok(wobu_store::project::narrative_generation::published(project, request, receipt_id, receipt)?)
}
