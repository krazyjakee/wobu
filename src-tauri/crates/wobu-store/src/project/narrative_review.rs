//! Canonical review evidence, guarded transitions and immutable history.
mod capture;
mod dependencies;
use dependencies::{capture_context, historical_context};
mod proposals;
mod write;
use super::Project;
use crate::{Error, Result, SceneFile, atomic::Stamp};
pub use capture::ReviewSnapshot;
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_narrative::review::{EditorialAction, ReviewContext, ReviewTarget};
use wobu_narrative::{Freshness, GenerationPolicy, ReviewState, SceneId, Speaker, Text};
pub(crate) use write::scene_lock;
pub use write::{ReviewTransaction, validate_manual};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewGuard {
    pub stamp: Option<Stamp>,
    pub head: Option<Id>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub guard: ReviewGuard,
    pub target: ReviewTarget,
    pub context_revision: String,
    pub state_json: String,
    pub action: EditorialAction,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewProposal {
    pub id: Id,
    pub hash: String,
    pub request_id: Id,
    pub receipt_id: Id,
    pub candidate: wobu_narrative_generation::Candidate,
    pub base_revision: Option<wobu_narrative::Revision>,
    pub base_wording: Option<String>,
    pub status: &'static str,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewLine {
    pub target: ReviewTarget,
    pub speaker: Speaker,
    pub text: Option<Text>,
    pub slot_policy: GenerationPolicy,
    pub review: ReviewState,
    pub freshness: Freshness,
    pub approval_valid: bool,
    pub reason: String,
    pub context_revision: String,
    pub proposals: Vec<ReviewProposal>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewHistory {
    pub id: Id,
    pub action: String,
    pub target: Option<ReviewTarget>,
    pub actor: String,
    pub context_revision: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewSceneView {
    pub scene_id: SceneId,
    pub guard: ReviewGuard,
    pub state_json: String,
    pub lines: Vec<ReviewLine>,
    pub history: Vec<ReviewHistory>,
    pub context_summary: String,
}
fn invalid(reason: impl Into<String>) -> Error {
    Error::Malformed { path: "narrative/review".into(), reason: reason.into() }
}
impl Project {
    /// Progress only: decisions are read from a verified canonical source/head
    /// chain. Actual acceptance still acquires its lock and rechecks all inputs.
    pub fn narrative_decided_proposals(
        &self,
        scenes: &std::collections::BTreeMap<SceneId, std::collections::BTreeSet<Id>>,
    ) -> Result<std::collections::BTreeSet<Id>> {
        let mut decided = std::collections::BTreeSet::new();
        for (id, requested) in scenes {
            let file = self.load_editorial_source(*id)?;
            let mut observations = std::collections::BTreeMap::new();
            let (history, problem) = capture::source_history(self, &file, &mut observations);
            if let Some(reason) = problem {
                return Err(invalid(reason));
            }
            if let Some(head) = history.first() {
                use wobu_narrative::review::ProposalDecision;
                let origins = history
                    .iter()
                    .filter_map(|event| {
                        let (id, hash, decision) = match &event.action {
                            EditorialAction::Generated { proposal_id, proposal_hash }
                            | EditorialAction::Accept { proposal_id, proposal_hash, .. } => {
                                (proposal_id, proposal_hash, ProposalDecision::Accepted)
                            }
                            EditorialAction::Reject { proposal_id, proposal_hash } => {
                                (proposal_id, proposal_hash, ProposalDecision::Rejected)
                            }
                            _ => return None,
                        };
                        Some((*id, (event, hash, decision)))
                    })
                    .collect::<std::collections::BTreeMap<_, _>>();
                for (proposal_id, decision) in
                    head.decisions.iter().filter(|(id, _)| requested.contains(id))
                {
                    let origin = origins.get(proposal_id).and_then(|(event, hash, expected)| {
                        (expected == decision).then_some((*event, *hash))
                    });
                    let Some((event, hash)) = origin else {
                        return Err(invalid(
                            "Proposal decision has no canonical editorial origin.",
                        ));
                    };
                    let proposal = proposals::checked(self, *proposal_id)?;
                    let target = &proposal.request.target;
                    if *hash != proposal.hash
                        || event.target.as_ref()
                            != Some(&ReviewTarget {
                                scene: target.scene,
                                beat: target.beat,
                                slot: target.slot,
                                variant: target.variant,
                            })
                    {
                        return Err(invalid(
                            "Editorial decision differs from its frozen proposal.",
                        ));
                    }
                    decided.insert(*proposal_id);
                }
            }
        }
        Ok(decided)
    }
    pub fn review_proposals(
        &self,
    ) -> Result<std::collections::BTreeMap<SceneId, Vec<ReviewProposal>>> {
        proposals::list(self)
    }
    pub fn review_scene(&self, id: SceneId, state: Option<&str>) -> Result<ReviewSceneView> {
        let snapshot = self.review_snapshot(id, state)?;
        snapshot.view(self)
    }
    pub fn review_context(
        &self,
        target: &ReviewTarget,
        state: Option<&str>,
    ) -> Result<ReviewContext> {
        self.review_snapshot(target.scene, state)?.context(target)
    }
    pub fn apply_review(
        &mut self,
        request: &ReviewRequest,
    ) -> Result<(SceneFile, ReviewSceneView)> {
        let mut transaction = self.begin_review(request)?;
        self.stage_review(&mut transaction, request)?;
        let file = self.commit_review(transaction)?;
        let view = self.review_scene(file.scene.id, Some(&request.state_json))?;
        Ok((file, view))
    }
}
