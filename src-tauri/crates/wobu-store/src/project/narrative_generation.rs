//! Exact immutable generation publication pair verification, shared by jobs and review.
use crate::{Error, NarrativeRecordKind, Project, Result};
use wobu_core::Id;
use wobu_narrative_generation::{
    AttemptStatus, FrozenRequest, Proposal, PublicationChecks, Receipt, VERSION,
};
fn invalid(message: impl std::fmt::Display) -> Error {
    Error::Malformed { path: "narrative/generation".into(), reason: message.to_string() }
}
pub fn proposal(
    request: &FrozenRequest,
    receipt_id: Id,
    candidate: wobu_narrative_generation::Candidate,
    publication_checks: PublicationChecks,
) -> Proposal {
    Proposal::NarrativeText {
        version: VERSION,
        request_id: request.request_id,
        receipt_id,
        request_hash: request.hash(),
        target: request.target.clone(),
        candidate,
        expected_scene_hash: request.expected_scene_hash.clone(),
        expected_text_revision: request.expected_text_revision.clone(),
        expected_policy: request.expected_policy,
        expected_slot_policy: request.expected_slot_policy,
        expected_context_hash: request.context.hash.clone(),
        publication_checks,
    }
}
/// Existing checks describe the original publication instant and are never recomputed here.
pub fn published(
    project: &Project,
    request: &FrozenRequest,
    receipt_id: Id,
    receipt: &Receipt,
) -> Result<Option<PublicationChecks>> {
    let Some(publication) = project.narrative_publication(receipt_id)? else { return Ok(None) };
    let invalid_pair = || {
        invalid(
            "Generation publication does not contain its exact receipt/proposal pair. Retained success will not be regenerated.",
        )
    };
    if publication.records.len() != 2 {
        return Err(invalid_pair());
    }
    let receipt_record = publication
        .records
        .iter()
        .find(|r| r.kind == NarrativeRecordKind::Receipt && r.id == receipt_id)
        .ok_or_else(invalid_pair)?;
    if receipt_record.payload != serde_json::to_value(receipt).map_err(invalid)? {
        return Err(invalid_pair());
    }
    let proposal_record = publication
        .records
        .iter()
        .find(|r| r.kind == NarrativeRecordKind::Proposal && r.id == receipt_id)
        .ok_or_else(invalid_pair)?;
    let found: Proposal =
        serde_json::from_value(proposal_record.payload.clone()).map_err(invalid)?;
    let Proposal::NarrativeText { publication_checks, .. } = &found;
    let Receipt::NarrativeGenerationAttempt {
        status: AttemptStatus::Succeeded,
        candidate: Some(candidate),
        raw_accepted_output: Some(raw),
        ..
    } = receipt
    else {
        return Err(invalid_pair());
    };
    if request.validate_output(raw).map_err(invalid)? != *candidate
        || found != proposal(request, receipt_id, candidate.clone(), publication_checks.clone())
    {
        return Err(invalid_pair());
    }
    Ok(Some(publication_checks.clone()))
}

/// Completed publications remain canonical even if a redundant standalone
/// attempt receipt is removed. Orphan objects never enter this set.
pub fn receipts(project: &Project) -> Result<Vec<crate::NarrativeRecordDocument>> {
    use std::collections::BTreeMap;
    let mut documents = BTreeMap::new();
    for record in project.narrative_records(NarrativeRecordKind::Receipt)? {
        documents.insert(record.document.id, record.document);
    }
    for record in
        published_records(project, NarrativeRecordKind::Receipt, "narrative_generation_attempt")?
    {
        if let Some(previous) = documents.insert(record.id, record.clone())
            && previous != record
        {
            return Err(invalid("Standalone and published generation receipts disagree."));
        }
    }
    // Validate each owning pair before exposing a recovered successful receipt.
    for record in documents.values() {
        if record.payload.get("type").and_then(serde_json::Value::as_str)
            != Some("narrative_generation_attempt")
        {
            continue;
        }
        let receipt: Receipt = serde_json::from_value(record.payload.clone())?;
        if let Receipt::NarrativeGenerationAttempt {
            request_id,
            status: AttemptStatus::Succeeded,
            ..
        } = &receipt
        {
            let source = documents
                .get(request_id)
                .ok_or_else(|| invalid("Published attempt has no frozen request."))?;
            let Receipt::NarrativeGenerationRequest { request } =
                serde_json::from_value(source.payload.clone())?
            else {
                return Err(invalid("Invalid generation request receipt."));
            };
            if project.narrative_publication(record.id)?.is_some() {
                published(project, &request, record.id, &receipt)?;
            }
        }
    }
    Ok(documents.into_values().collect())
}

/// Only complete manifests make generated candidates visible. A standalone
/// mutable proposal cannot stand in for the exact validated immutable pair.
pub fn proposal_documents(project: &Project) -> Result<Vec<crate::NarrativeRecordDocument>> {
    published_records(project, NarrativeRecordKind::Proposal, "narrative_text")
}
fn published_records(
    project: &Project,
    kind: NarrativeRecordKind,
    tag: &str,
) -> Result<Vec<crate::NarrativeRecordDocument>> {
    use crate::narrative::registry;
    let mut documents = std::collections::BTreeMap::new();
    for (rel, _) in registry::paths(project.root())? {
        if registry::classify(&rel) != Some(registry::NarrativeFileKind::Publication) {
            continue;
        }
        let Some((text, _)) = registry::read(project.root(), &rel)? else { continue };
        let (_, _, value) = registry::parse(&rel, &text)?;
        let manifest: crate::narrative::publication::NarrativePublication =
            serde_json::from_value(value)?;
        let Some(publication) = project.narrative_publication(manifest.id)? else { continue };
        for record in publication.records {
            if record.kind != kind
                || record.payload.get("type").and_then(serde_json::Value::as_str) != Some(tag)
            {
                continue;
            }
            if let Some(previous) = documents.insert(record.id, record.clone())
                && previous != record
            {
                return Err(invalid("Published record identities disagree."));
            }
        }
    }
    Ok(documents.into_values().collect())
}
