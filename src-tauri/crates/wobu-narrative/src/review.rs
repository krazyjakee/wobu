//! Review evidence is distinct from generation provenance and from writable UI flags.
use crate::{
    BeatId, DialogueSlotId, GenerationPolicy, Name, Revision, Scene, SceneId, Speaker, Text, Value,
    VariantId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use wobu_core::Id;

pub const REVIEW_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTarget {
    pub scene: SceneId,
    pub beat: BeatId,
    pub slot: DialogueSlotId,
    pub variant: Option<VariantId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewContext {
    pub version: u32,
    pub revision: String,
    pub state: BTreeMap<Name, Value>,
    pub inputs: Json,
}

pub fn hash(value: &impl Serialize) -> String {
    blake3::hash(&serde_json::to_vec(value).expect("narrative data serializes"))
        .to_hex()
        .to_string()
}

impl ReviewContext {
    /// Full conservative authored context, without editorial metadata or the
    /// reviewed wording. That wording has its own exact identity/revision binding.
    pub fn capture(
        scene: &Scene,
        target: &ReviewTarget,
        world: Json,
        schema: Json,
        characters: Json,
        state: BTreeMap<Name, Value>,
    ) -> Self {
        let mut source = serde_json::to_value(scene).expect("scene serializes");
        source.as_object_mut().unwrap().remove("editorial_head");
        if let Some(beats) = source["beats"].as_array_mut() {
            for beat in beats {
                let selected_beat = beat["id"] == target.beat.to_string();
                if let Some(slots) = beat["dialogue"].as_array_mut() {
                    for slot in slots {
                        let selected_slot = selected_beat && slot["id"] == target.slot.to_string();
                        slot.as_object_mut().unwrap().remove("policy");
                        if let Some(variants) = slot["variants"].as_array_mut() {
                            for variant in variants {
                                let selected = selected_slot
                                    && target
                                        .variant
                                        .is_some_and(|id| variant["id"] == id.to_string());
                                if let Some(text) = variant["text"].as_object_mut() {
                                    text.remove("lifecycle");
                                    if selected {
                                        text.remove("body");
                                        text.remove("revision");
                                        text.remove("provenance");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let inputs = json!({"target":target,"scene":source,"world":world,"schema":schema,"characters":characters});
        let revision = hash(&(REVIEW_VERSION, &inputs, &state));
        Self { version: REVIEW_VERSION, revision, state, inputs }
    }
    pub fn valid(&self) -> bool {
        self.version == REVIEW_VERSION
            && self.revision == hash(&(self.version, &self.inputs, &self.state))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBinding {
    pub target: ReviewTarget,
    pub speaker: Speaker,
    pub text_revision: Revision,
    pub context_revision: String,
    pub state: BTreeMap<Name, Value>,
    pub approved: bool,
    pub event_id: Id,
}
impl ReviewBinding {
    pub fn matches(&self, target: &ReviewTarget, speaker: &Speaker, text: &Text) -> bool {
        &self.target == target
            && &self.speaker == speaker
            && self.text_revision == text.revision
            && text.revision_matches()
            && self.context_revision.len() == 64
    }
}

/// Explicit host-supplied domain evidence for the pure compiler. Raw scene flags
/// cannot construct a proof; the host loads and verifies immutable review history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalEvidence {
    pub binding: ReviewBinding,
    pub current_context: String,
}
impl ApprovalEvidence {
    pub fn verifies(&self, target: &ReviewTarget, speaker: &Speaker, text: &Text) -> bool {
        self.binding.approved
            && self.binding.matches(target, speaker, text)
            && self.binding.context_revision == self.current_context
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyScope {
    Slot,
    Variant,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditorialAction {
    ManualSave,
    Restore,
    Edit { body: String },
    Policy { scope: PolicyScope, policy: GenerationPolicy },
    Approve,
    Attest,
    Accept { proposal_id: Id, proposal_hash: String, reviewed_text: Option<String> },
    Reject { proposal_id: Id, proposal_hash: String },
    Generated { proposal_id: Id, proposal_hash: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalDecision {
    Accepted,
    Rejected,
}

/// Stored inside the existing immutable Receipt envelope. Only the event named
/// by canonical scene.editorial_head and its ancestors are committed history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditorialEvent {
    #[serde(rename = "type")]
    pub record_type: String,
    pub version: u32,
    pub id: Id,
    pub parent: Option<Id>,
    pub target: Option<ReviewTarget>,
    pub action: EditorialAction,
    pub actor: String,
    pub context: Option<ReviewContext>,
    pub before: Scene,
    /// Head cleared to avoid a self-reference; publication installs this event's id.
    pub after: Scene,
    pub bindings: BTreeMap<VariantId, ReviewBinding>,
    pub decisions: BTreeMap<Id, ProposalDecision>,
}
impl EditorialEvent {
    pub fn valid(&self) -> bool {
        self.record_type == "narrative_editorial"
            && self.version == REVIEW_VERSION
            && self.before.id == self.after.id
            && self.after.editorial_head.is_none()
            && self.before.editorial_head == self.parent
            && !self.actor.trim().is_empty()
            && self.target.as_ref().is_none_or(|t| t.scene == self.after.id)
            && self.bindings.iter().all(|(id, b)| {
                b.target.variant == Some(*id)
                    && b.target.scene == self.after.id
                    && b.context_revision.len() == 64
            })
    }
}
