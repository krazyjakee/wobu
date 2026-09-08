use super::*;
use crate::NarrativeRecordKind;
use wobu_narrative::review::hash;
use wobu_narrative_generation::{FrozenRequest, Proposal, Receipt};

pub(crate) struct CheckedProposal {
    pub request: FrozenRequest,
    pub candidate: wobu_narrative_generation::Candidate,
    pub hash: String,
}
pub(crate) fn checked(project: &Project, id: Id) -> Result<CheckedProposal> {
    let publication = project
        .narrative_publication(id)?
        .ok_or_else(|| invalid("Proposal publication is incomplete or missing."))?;
    let payload = publication
        .records
        .iter()
        .find(|r| r.kind == NarrativeRecordKind::Proposal && r.id == id)
        .ok_or_else(|| invalid("Proposal record is missing."))?
        .payload
        .clone();
    let Proposal::NarrativeText { request_id, receipt_id, candidate, .. } =
        serde_json::from_value(payload.clone())?;
    if receipt_id != id {
        return Err(invalid("Proposal receipt identity does not match."));
    }
    let request_file = project
        .narrative_record(NarrativeRecordKind::Receipt, request_id)?
        .ok_or_else(|| invalid("Frozen generation request is missing."))?;
    let Receipt::NarrativeGenerationRequest { request } =
        serde_json::from_value(request_file.document.payload)?
    else {
        return Err(invalid("Record is not a frozen generation request."));
    };
    request.validate().map_err(invalid)?;
    if request.request_id != request_id {
        return Err(invalid("Request receipt identity does not match."));
    }
    let receipt_record = publication
        .records
        .iter()
        .find(|r| r.kind == NarrativeRecordKind::Receipt && r.id == id)
        .ok_or_else(|| invalid("Generation attempt receipt is missing."))?;
    let receipt: Receipt = serde_json::from_value(receipt_record.payload.clone())?;
    let Receipt::NarrativeGenerationAttempt { request_id: rid, request_hash, .. } = &receipt else {
        return Err(invalid("Proposal requires a generation attempt."));
    };
    if *rid != request_id || *request_hash != request.hash() {
        return Err(invalid("Generation attempt does not match frozen request."));
    }
    super::super::narrative_generation::published(project, &request, id, &receipt)?
        .ok_or_else(|| invalid("Incomplete proposal publication."))?;
    Ok(CheckedProposal { request: *request, candidate, hash: hash(&payload) })
}
pub(crate) fn list(
    project: &Project,
) -> Result<std::collections::BTreeMap<SceneId, Vec<ReviewProposal>>> {
    let mut result = std::collections::BTreeMap::<SceneId, Vec<ReviewProposal>>::new();
    for record in super::super::narrative_generation::proposal_documents(project)? {
        if record.payload.get("type").and_then(serde_json::Value::as_str) != Some("narrative_text")
        {
            continue;
        }
        let Proposal::NarrativeText { target, .. } = serde_json::from_value(record.payload)?;
        let checked = checked(project, record.id)?;
        let status = "pending";
        let base_wording = checked
            .request
            .context
            .fragments
            .iter()
            .find(|f| f.kind == "existing_wording")
            .and_then(|f| f.data.get("body"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        result.entry(target.scene).or_default().push(ReviewProposal {
            id: record.id,
            hash: checked.hash,
            request_id: checked.request.request_id,
            receipt_id: record.id,
            candidate: checked.candidate,
            base_revision: checked.request.expected_text_revision,
            base_wording,
            status,
            reason: if status == "pending" {
                "Acceptance rechecks the original wording, policies and frozen context.".into()
            } else {
                format!("Proposal {status} in canonical editorial history.")
            },
        });
    }
    Ok(result)
}
pub(crate) fn unchanged(
    project: &Project,
    snapshot: &ReviewSnapshot,
    proposal: &CheckedProposal,
) -> Result<()> {
    let request = &proposal.request;
    if !super::super::narrative_generation::source_unchanged(&snapshot.file, request) {
        return Err(invalid(
            "Scene changed since this proposal was requested. Generate a new proposal or edit the current wording explicitly.",
        ));
    }
    let current =
        super::super::narrative_context::capture(project, request.context.options.clone(), || {})?;
    if !wobu_narrative_generation::context_matches(request, &current) {
        return Err(invalid(
            "Generation context changed. The retained proposal cannot replace current wording.",
        ));
    }
    Ok(())
}
