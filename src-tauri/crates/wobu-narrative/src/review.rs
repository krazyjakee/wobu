//! Review evidence is distinct from generation provenance and from writable UI flags.
use crate::{
    BeatId, DialogueSlotId, GenerationPolicy, Name, Revision, Scene, SceneId, Speaker, Text,
    TextAsset, TextAssetId, TextEntryId, Value, VariantId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use wobu_core::Id;

pub const REVIEW_VERSION: u32 = 1;
/// Current context fingerprint contract; editorial envelope version remains stable.
pub const REVIEW_CONTEXT_VERSION: u32 = 2;

/// Strip a serialized document down to what a reviewer actually read.
///
/// Removes every writable lifecycle flag, and removes the reviewed wording
/// itself — its body, revision and provenance — because that wording has its own
/// exact revision binding and including it here would make the context hash
/// change every time the line was edited, withdrawing approvals from every
/// *other* line in the document.
///
/// Written once over `(section, line)` key names so a scene and a supporting
/// text asset are stripped identically. Two copies of this walk would be the
/// first place the two could diverge, and a divergence here is an approval that
/// means something different depending on which editor produced it.
fn strip_reviewed(
    source: &mut Json,
    sections: &str,
    lines: &str,
    selected_section: &str,
    selected_slot: &DialogueSlotId,
    selected_variant: Option<VariantId>,
) {
    let Some(entries) = source[sections].as_array_mut() else { return };
    for entry in entries {
        let here = entry["id"] == selected_section;
        let Some(slots) = entry[lines].as_array_mut() else { continue };
        for slot in slots {
            let selected_slot = here && slot["id"] == selected_slot.to_string();
            slot.as_object_mut().expect("a slot is an object").remove("policy");
            let Some(variants) = slot["variants"].as_array_mut() else { continue };
            for variant in variants {
                let selected = selected_slot
                    && selected_variant.is_some_and(|id| variant["id"] == id.to_string());
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
        if let Some(asset) = scene.editorial_text() {
            return Self::capture_text(
                &asset,
                &TextTarget {
                    asset: asset.id,
                    entry: TextEntryId::from_raw(target.beat.raw()),
                    slot: target.slot,
                    variant: target.variant,
                },
                world,
                schema,
                characters,
                state,
            );
        }
        let mut source = serde_json::to_value(scene).expect("scene serializes");
        source.as_object_mut().expect("a scene is an object").remove("editorial_head");
        strip_reviewed(
            &mut source,
            "beats",
            "dialogue",
            &target.beat.to_string(),
            &target.slot,
            target.variant,
        );
        let inputs = json!({"target":target,"scene":source,"world":world,"schema":schema,"characters":characters});
        let revision = hash(&(REVIEW_VERSION, &inputs, &state));
        Self { version: REVIEW_VERSION, revision, state, inputs }
    }

    /// The same conservative capture for a supporting text asset (#167).
    ///
    /// A second constructor rather than a second context type, because what an
    /// approval attests to is identical for a bark and for a scene line: the
    /// authored surroundings the reviewer read, with the reviewed wording itself
    /// removed because that wording has its own exact revision binding. Giving
    /// supporting text its own context record would be the first place the two
    /// could quietly diverge, and a divergence here is an approval that means
    /// something different depending on which editor produced it.
    pub fn capture_text(
        asset: &TextAsset,
        target: &TextTarget,
        world: Json,
        schema: Json,
        characters: Json,
        state: BTreeMap<Name, Value>,
    ) -> Self {
        let mut source = serde_json::to_value(asset).expect("text asset serializes");
        source.as_object_mut().expect("text asset is an object").remove("editorial_head");
        source.as_object_mut().expect("text asset is an object").remove("policy");
        strip_reviewed(
            &mut source,
            "entries",
            "lines",
            &target.entry.to_string(),
            &target.slot,
            target.variant,
        );
        let inputs = json!({"target":target,"text":source,"world":world,"schema":schema,"characters":characters});
        let revision = hash(&(REVIEW_VERSION, &inputs, &state));
        Self { version: REVIEW_VERSION, revision, state, inputs }
    }

    /// Version 2 binds only the target's dependency inputs and referenced state
    /// values. Full frozen inputs remain in the receipt; v1 verification is kept.
    pub fn with_dependencies(mut self, dependencies: Json) -> Self {
        self.version = REVIEW_CONTEXT_VERSION;
        self.inputs["dependencies"] = dependencies;
        self.revision = self.dependency_revision();
        self
    }

    fn dependency_revision(&self) -> String {
        let fields = self.inputs["dependencies"]["fields"].as_object();
        let state: BTreeMap<_, _> = self
            .state
            .iter()
            .filter(|(name, _)| {
                fields.is_some_and(|fields| fields.contains_key(&format!("state/{name}")))
            })
            .collect();
        hash(&(self.version, &self.inputs["dependencies"], state))
    }

    pub fn valid(&self) -> bool {
        match self.version {
            1 => self.revision == hash(&(self.version, &self.inputs, &self.state)),
            2 => {
                self.inputs["dependencies"].is_object()
                    && self.revision == self.dependency_revision()
            }
            _ => false,
        }
    }
}

/// Which line of a supporting text asset a review decision is about.
///
/// The same four levels as [`ReviewTarget`] — container, section, slot, wording
/// — over the identities supporting text actually has. It is a sibling type
/// rather than a fifth field on `ReviewTarget`, because a target that could name
/// a scene and an asset at once is a state no editor can produce and every
/// consumer would have to reject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextTarget {
    pub asset: TextAssetId,
    pub entry: TextEntryId,
    pub slot: DialogueSlotId,
    pub variant: Option<VariantId>,
}

/// The one rule an approval has to satisfy, for every kind of authored line.
///
/// Written once and called from both [`ReviewBinding::matches`] and
/// [`TextBinding::matches`], so supporting text cannot drift into a weaker
/// check than scene dialogue. The rule is: the approval names this exact
/// speaker and this exact wording revision, the stored revision still describes
/// the stored words, and the context revision is a real digest rather than a
/// placeholder somebody filled in.
fn wording_is_bound(
    binding_speaker: &Speaker,
    binding_revision: &Revision,
    context_revision: &str,
    speaker: &Speaker,
    text: &Text,
) -> bool {
    binding_speaker == speaker
        && binding_revision == &text.revision
        && text.revision_matches()
        && context_revision.len() == 64
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
            && wording_is_bound(
                &self.speaker,
                &self.text_revision,
                &self.context_revision,
                speaker,
                text,
            )
    }
}

/// The supporting-text counterpart of [`ReviewBinding`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBinding {
    pub target: TextTarget,
    pub speaker: Speaker,
    pub text_revision: Revision,
    pub context_revision: String,
    pub state: BTreeMap<Name, Value>,
    pub approved: bool,
    pub event_id: Id,
}
impl TextBinding {
    pub fn matches(&self, target: &TextTarget, speaker: &Speaker, text: &Text) -> bool {
        &self.target == target
            && wording_is_bound(
                &self.speaker,
                &self.text_revision,
                &self.context_revision,
                speaker,
                text,
            )
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

/// The supporting-text counterpart of [`ApprovalEvidence`].
///
/// Release compilation refuses a bark whose approval is missing, is against
/// other wording, or was given against a context that has since moved — exactly
/// the three refusals scene dialogue already gets. A writable `Approved` flag in
/// the asset file still cannot authorise a release on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextApprovalEvidence {
    pub binding: TextBinding,
    pub current_context: String,
}
impl TextApprovalEvidence {
    pub fn verifies(&self, target: &TextTarget, speaker: &Speaker, text: &Text) -> bool {
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
    Materialize { report_id: Id },
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
