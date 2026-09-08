//! Canonical review evidence, guarded transitions and immutable history.
mod capture;
mod proposals;
mod write;
use super::Project;
use crate::{Error, Result, SceneFile, atomic::Stamp};
pub use capture::ReviewSnapshot;
use serde::{Deserialize, Serialize};
use wobu_core::Id;
use wobu_narrative::review::{EditorialAction, ReviewContext, ReviewTarget};
use wobu_narrative::{Freshness, GenerationPolicy, ReviewState, SceneId, Speaker, Text};
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
