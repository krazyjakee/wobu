//! The scene: participants, ordered beats, choices, outcomes and dialogue.
//!
//! One thing that is conspicuously absent, and stays absent: there is no `x`,
//! no `y`, no `collapsed`, no port ordering and no colour anywhere in this
//! module. Where a beat sits on the Flow canvas is presentation metadata stored
//! beside the source (#185), because moving a box must not change story
//! behaviour, revisions or freshness — and a coordinate in this file would do
//! exactly that: it would land in the diff a collaborator merges, in the hash a
//! translation is keyed to, and in the payload the game ships. The pull to add
//! "just a position, it is only two numbers" is the reason #185 exists.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::expr::{Condition, Effect};
use crate::id::{
    BeatId, ChoiceId, DialogueSlotId, OutcomeId, Provenance, Revision, SceneId, VariantId,
};
use crate::lifecycle::{ContentLifecycle, GenerationPolicy};

/// A world entity — a character, a faction, a place — as it already exists in
/// the project.
///
/// An alias for `wobu_core::Id` rather than a new identity type, and that is the
/// point of this crate depending on `wobu-core` at all: a scene's participants
/// are the project's characters. Copying them into a narrative-only character
/// record would create a second world that drifts from the first, and the first
/// symptom would be a generated line describing a character's voice from before
/// somebody changed it.
pub type EntityId = wobu_core::Id;

/// Who is speaking, or whose intent is being described.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Speaker {
    /// The player character. Not an entity, because the player is not a row in
    /// the world model and giving them a synthetic id would put them in the
    /// character list.
    Player,
    /// Unattributed narration or stage direction.
    Narrator,
    Entity(EntityId),
}

impl Speaker {
    /// The world entity behind this speaker, if there is one.
    pub fn entity(&self) -> Option<EntityId> {
        match self {
            Speaker::Entity(id) => Some(*id),
            Speaker::Player | Speaker::Narrator => None,
        }
    }
}

/// One character taking part in a scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Participant {
    pub entity: EntityId,
    /// What they are here as — "accuser", "witness". Free text: it labels the
    /// participant for a writer reading the scene, and nothing branches on it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
}

/// What one participant is trying to do in a beat.
///
/// Prose, and deliberately inert. #151 requires that prose cannot author
/// executable effects, so an intent is never parsed, never matched and never
/// consulted by anything that decides where the story goes; it is context for a
/// writer and, later, for a generation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Intent {
    pub subject: Speaker,
    pub intent: String,
}

/// A piece of authored wording, and the revision that identifies it.
///
/// The revision is stored rather than recomputed on read, because it is what a
/// translation, a recording and an approval are all keyed to (#178, #179), and
/// a value recomputed on read would silently agree with whatever the file
/// happened to say. Storing it means a hand-edit in the Source view can leave
/// the two disagreeing — which is reported as a diagnostic, not repaired in
/// passing, because repairing it is what would break the locale pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Text {
    pub revision: Revision,
    pub body: String,
    #[serde(default)]
    pub provenance: Provenance,
    #[serde(default)]
    pub lifecycle: ContentLifecycle,
}

impl Text {
    /// Wording somebody typed.
    pub fn written(body: impl Into<String>) -> Text {
        let body = body.into();
        Text {
            revision: Revision::of(&body, &Provenance::Human),
            body,
            provenance: Provenance::Human,
            lifecycle: ContentLifecycle::hand_written(),
        }
    }

    /// Wording somebody typed and locked in the same breath — the writer in
    /// US-03 who marks a line final without ever configuring an LLM.
    pub fn written_locked(body: impl Into<String>) -> Text {
        Text { lifecycle: ContentLifecycle::hand_written_locked(), ..Text::written(body) }
    }

    /// Rewrite the words.
    ///
    /// A new revision, the same slot and the same variant id — this is the
    /// operation the whole id/revision split exists for. The review state falls
    /// back to draft because an approval was given to the previous wording and
    /// nobody has looked at this one.
    pub fn set_body(&mut self, body: impl Into<String>, provenance: Provenance) -> &Revision {
        self.body = body.into();
        self.provenance = provenance;
        self.revision = Revision::of(&self.body, &self.provenance);
        self.lifecycle.review = crate::lifecycle::ReviewState::Draft;
        &self.revision
    }

    /// The revision this body and provenance actually hash to.
    pub fn computed_revision(&self) -> Revision {
        Revision::of(&self.body, &self.provenance)
    }

    /// Whether the stored revision still describes the stored words.
    pub fn revision_matches(&self) -> bool {
        self.revision == self.computed_revision()
    }

    /// Adopt the revision the current words hash to.
    ///
    /// The explicit repair for a hand-edited source file. Explicit because
    /// everything keyed to the old revision — translations, recordings, an
    /// approval — is about to stop matching, and that is a decision somebody
    /// should make on purpose.
    pub fn reseal(&mut self) -> &Revision {
        self.revision = self.computed_revision();
        &self.revision
    }
}

/// One conditioned wording for a dialogue slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Variant {
    pub id: VariantId,
    /// Which state this wording is for. `None` is the unconditional variant —
    /// the one used when nothing more specific applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    pub text: Text,
}

impl Variant {
    pub fn new(text: Text) -> Variant {
        Variant { id: VariantId::new(), when: None, text }
    }

    /// A copy in a new slot: new identity, same words.
    ///
    /// The revision is deliberately carried over rather than re-derived. It
    /// would come out the same either way, and that is the point worth making:
    /// the revision describes the wording, so copying the wording copies the
    /// revision, while the identity — the thing a recording is filed under — is
    /// new because this is a different line.
    pub fn duplicated(&self) -> Variant {
        Variant { id: VariantId::new(), ..self.clone() }
    }
}

/// A named place a line of dialogue goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DialogueSlot {
    pub id: DialogueSlotId,
    pub speaker: Speaker,
    /// Whether generation may write into this slot at all.
    ///
    /// Distinct from the policy on each variant's [`Text`], and the two answer
    /// different questions. This one is a *rule about the slot*: locking it
    /// stops any future job touching it, which is what US-05 needs to hold even
    /// for a caller that bypasses the UI. The one on a variant is a *fact about
    /// that wording*: who wrote it and whether it may be replaced. Folding them
    /// together would mean locking a slot rewrote the record of how its existing
    /// lines came to be.
    #[serde(default)]
    pub policy: GenerationPolicy,
    /// Ordered wordings. An empty list is a slot with no text yet — which is
    /// shown as a task rather than filled in on save (US-02).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Variant>,
}

impl DialogueSlot {
    pub fn new(speaker: Speaker) -> DialogueSlot {
        DialogueSlot {
            id: DialogueSlotId::new(),
            speaker,
            policy: GenerationPolicy::default(),
            variants: Vec::new(),
        }
    }

    /// Whether this slot is still waiting for words.
    pub fn is_missing_text(&self) -> bool {
        self.variants.is_empty()
    }

    /// Whether a generation job may write here. Both the slot's own rule and
    /// every existing variant have to allow it.
    pub fn may_generate(&self) -> bool {
        self.policy != GenerationPolicy::Locked
            && self.variants.iter().all(|v| v.text.lifecycle.may_generate())
    }

    pub fn duplicated(&self) -> DialogueSlot {
        DialogueSlot {
            id: DialogueSlotId::new(),
            speaker: self.speaker.clone(),
            policy: self.policy,
            variants: self.variants.iter().map(Variant::duplicated).collect(),
        }
    }
}

/// Where the story goes next.
///
/// Three cases and no fourth. In particular there is no "fall through to the
/// next beat in the list": an implicit destination would mean reordering beats
/// silently rewired the story, which is precisely the class of accident stable
/// ids and explicit destinations exist to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Destination {
    /// Another beat in this same scene.
    Beat(BeatId),
    /// A different scene. Whether it exists is not knowable from one file, which
    /// is why [`SceneCatalog`](crate::SceneCatalog) has to be handed in to check
    /// it.
    Scene(SceneId),
    /// The story stops here, deliberately. `label` names the ending for an
    /// export and for a reader; it may be empty, but the *end* itself is always
    /// written down — an unfinished branch and a finished one must not look the
    /// same.
    End {
        #[serde(default)]
        label: String,
    },
}

/// A player-facing branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Choice {
    pub id: ChoiceId,
    /// What the player sees on the button.
    pub label: String,
    /// When this choice is offered at all. `None` is the "always" row of the
    /// choice table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    pub to: Destination,
}

impl Choice {
    pub fn new(label: impl Into<String>, to: Destination) -> Choice {
        Choice { id: ChoiceId::new(), label: label.into(), requires: None, effects: Vec::new(), to }
    }

    pub fn duplicated(&self) -> Choice {
        Choice { id: ChoiceId::new(), ..self.clone() }
    }
}

/// An automatic transition out of a beat: what happens when the player is not
/// being asked.
///
/// Ordered within a beat, and the order is the author's stated priority. This
/// model only records it; deciding a winner is the compiler's and the runtime's
/// job (#158, #159).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Outcome {
    pub id: OutcomeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    pub to: Destination,
}

impl Outcome {
    pub fn new(to: Destination) -> Outcome {
        Outcome { id: OutcomeId::new(), when: None, effects: Vec::new(), to }
    }

    pub fn duplicated(&self) -> Outcome {
        Outcome { id: OutcomeId::new(), ..self.clone() }
    }
}

/// One beat: a unit of intent, dialogue and branching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Beat {
    pub id: BeatId,
    /// The beat's name. A display string: renaming it changes nothing else, and
    /// no id anywhere is derived from it.
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intents: Vec<Intent>,
    /// What this beat has to get across. Prose, and inert — see [`Intent`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_convey: Vec<String>,
    /// What must not be revealed here. Also prose, and also inert: this is a
    /// brief for a writer and an instruction in a generation request, and it is
    /// worth being blunt that a sentence in this list is not a proof that a
    /// model never said the thing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_reveal: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogue: Vec<DialogueSlot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<Outcome>,
}

impl Beat {
    pub fn new(title: impl Into<String>) -> Beat {
        Beat {
            id: BeatId::new(),
            title: title.into(),
            intents: Vec::new(),
            must_convey: Vec::new(),
            must_not_reveal: Vec::new(),
            dialogue: Vec::new(),
            choices: Vec::new(),
            outcomes: Vec::new(),
        }
    }

    /// Every destination this beat states, with the choice or outcome
    /// responsible for it.
    pub fn destinations(&self) -> impl Iterator<Item = (DestinationSite, &Destination)> {
        let choices = self
            .choices
            .iter()
            .map(move |c| (DestinationSite::Choice { beat: self.id, choice: c.id }, &c.to));
        let outcomes = self
            .outcomes
            .iter()
            .map(move |o| (DestinationSite::Outcome { beat: self.id, outcome: o.id }, &o.to));
        choices.chain(outcomes)
    }

    /// Whether this beat states any way out at all.
    pub fn has_destination(&self) -> bool {
        !self.choices.is_empty() || !self.outcomes.is_empty()
    }

    /// A copy: fresh identities all the way down, everything else preserved.
    ///
    /// Destinations are copied verbatim, including one that points at the beat
    /// being copied. That is the honest reading of "duplicate this beat" — the
    /// copy branches where the original branched — and rewriting a self-reference
    /// to point at the copy would be a guess about intent that the author cannot
    /// see us make. [`Scene::duplicated`] is different, and says why.
    pub fn duplicated(&self) -> Beat {
        Beat {
            id: BeatId::new(),
            title: self.title.clone(),
            intents: self.intents.clone(),
            must_convey: self.must_convey.clone(),
            must_not_reveal: self.must_not_reveal.clone(),
            dialogue: self.dialogue.iter().map(DialogueSlot::duplicated).collect(),
            choices: self.choices.iter().map(Choice::duplicated).collect(),
            outcomes: self.outcomes.iter().map(Outcome::duplicated).collect(),
        }
    }
}

/// The element a destination belongs to.
///
/// Every destination diagnostic is keyed by one of these, so a message about a
/// broken link can be turned straight into a selection: the responsible field in
/// the choice table, or the responsible node on the canvas (#186, #189).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DestinationSite {
    Choice {
        beat: BeatId,
        choice: ChoiceId,
    },
    Outcome {
        beat: BeatId,
        outcome: OutcomeId,
    },
    /// The beat itself, for the one destination problem that is not attributable
    /// to any choice or outcome: not having one.
    Beat(BeatId),
}

impl DestinationSite {
    pub fn beat(self) -> BeatId {
        match self {
            DestinationSite::Choice { beat, .. }
            | DestinationSite::Outcome { beat, .. }
            | DestinationSite::Beat(beat) => beat,
        }
    }
}

/// What a deleted element was, so a reference to it can still be explained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TombstoneTarget {
    Beat(BeatId),
    DialogueSlot(DialogueSlotId),
    Variant(VariantId),
}

/// The record left behind by a deletion.
///
/// Without one, a destination pointing at a removed beat is indistinguishable
/// from a destination pointing at a beat that never existed — a typed id and
/// nothing else — and the only diagnostic possible is "unknown beat 01J8…",
/// which tells the author nothing about what to do. With one, the same
/// diagnostic can say that "Verdict" was deleted, when, and why.
///
/// It also outlives the source: a locale pack or a recording script filed
/// against a variant id has to be explainable after the line is gone (#178,
/// #179), which is why slots and variants get tombstones and choices and
/// outcomes do not — nothing outside this file refers to those by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Tombstone {
    pub target: TombstoneTarget,
    /// The last human-readable name. The whole reason a tombstone is worth
    /// storing: a diagnostic naming "Verdict" is actionable, one naming a ULID
    /// is not.
    pub label: String,
    pub deleted_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One authored scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Scene {
    pub id: SceneId,
    /// The display name. Renaming it preserves every id in the file; nothing is
    /// derived from it.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<Participant>,
    /// When this scene may begin. `None` means unconditional, which is a
    /// different statement from `Some(Condition::Never)` — an author closing a
    /// scene off deliberately — and collapsing the two would make that
    /// unsayable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<Condition>,
    /// Beats in author order.
    ///
    /// The `Vec` is the order. There is no `order` field on [`Beat`], because a
    /// stored index can disagree with the position it is stored at, and then two
    /// readers of the same file disagree about what the scene is. Reordering is
    /// [`Scene::reorder_beat`] and preserves every id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beats: Vec<Beat>,
    /// Deletions, kept so broken references can be explained. See [`Tombstone`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstones: Vec<Tombstone>,
}

impl Scene {
    pub fn new(name: impl Into<String>) -> Scene {
        Scene {
            id: SceneId::new(),
            name: name.into(),
            summary: String::new(),
            participants: Vec::new(),
            entry: None,
            beats: Vec::new(),
            tombstones: Vec::new(),
        }
    }

    pub fn beat(&self, id: BeatId) -> Option<&Beat> {
        self.beats.iter().find(|b| b.id == id)
    }

    pub fn beat_mut(&mut self, id: BeatId) -> Option<&mut Beat> {
        self.beats.iter_mut().find(|b| b.id == id)
    }

    pub fn beat_index(&self, id: BeatId) -> Option<usize> {
        self.beats.iter().position(|b| b.id == id)
    }

    pub fn has_beat(&self, id: BeatId) -> bool {
        self.beats.iter().any(|b| b.id == id)
    }

    /// Move a beat to a new position, keeping every id and every destination
    /// exactly as it was.
    ///
    /// Returns whether the beat was found. `to` is clamped rather than rejected:
    /// a drag past the end of the list means "last", and failing the whole
    /// operation because a pointer went one row too far would be a worse answer
    /// than the obvious one.
    pub fn reorder_beat(&mut self, id: BeatId, to: usize) -> bool {
        let Some(from) = self.beat_index(id) else { return false };
        let to = to.min(self.beats.len().saturating_sub(1));
        let beat = self.beats.remove(from);
        self.beats.insert(to, beat);
        true
    }

    /// Copy a beat in beside the original.
    ///
    /// Returns the new beat's id. Everything under it — choices, outcomes,
    /// dialogue slots, variants — gets a fresh identity, because a duplicate is
    /// a second slot rather than a second name for the first one, and sharing an
    /// id would mean recording audio for one line and having it appear in two.
    pub fn duplicate_beat(&mut self, id: BeatId) -> Option<BeatId> {
        let index = self.beat_index(id)?;
        let copy = self.beats[index].duplicated();
        let new_id = copy.id;
        self.beats.insert(index + 1, copy);
        Some(new_id)
    }

    /// Remove a beat, leaving tombstones for it and for everything under it that
    /// something outside this file might still name.
    ///
    /// Destinations pointing at the removed beat are deliberately *not* rewired
    /// or cleared. Silently repairing them would delete the author's statement
    /// about where that branch went, and leave nothing to undo; leaving them
    /// broken is what turns the deletion into a diagnostic that names the beat
    /// that used to be there.
    pub fn remove_beat(&mut self, id: BeatId, reason: Option<String>) -> Option<Beat> {
        let index = self.beat_index(id)?;
        let beat = self.beats.remove(index);
        let deleted_at = Utc::now();

        self.tombstones.push(Tombstone {
            target: TombstoneTarget::Beat(beat.id),
            label: beat.title.clone(),
            deleted_at,
            reason: reason.clone(),
        });
        for slot in &beat.dialogue {
            self.tombstones.push(Tombstone {
                target: TombstoneTarget::DialogueSlot(slot.id),
                label: beat.title.clone(),
                deleted_at,
                reason: reason.clone(),
            });
            for variant in &slot.variants {
                self.tombstones.push(Tombstone {
                    target: TombstoneTarget::Variant(variant.id),
                    // The words themselves, because that is what identifies a
                    // line to the person holding a recording script.
                    label: variant.text.body.clone(),
                    deleted_at,
                    reason: reason.clone(),
                });
            }
        }
        Some(beat)
    }

    /// The tombstone for a deleted beat, if there is one.
    pub fn beat_tombstone(&self, id: BeatId) -> Option<&Tombstone> {
        self.tombstones.iter().find(|t| t.target == TombstoneTarget::Beat(id))
    }

    /// Copy the whole scene: a new scene id, new ids throughout, and internal
    /// destinations rewired to the copies.
    ///
    /// The rewiring is the difference from [`Beat::duplicated`], and it is
    /// justified by what "duplicate this scene" means: the copy has to be a
    /// standalone scene that plays the way the original does. Leaving its beats
    /// pointing at the original's beats would produce something that reads as a
    /// copy and behaves as a trapdoor into the original. Links *out* of the
    /// scene are left alone — they name scenes this operation is not copying.
    ///
    /// Tombstones are not carried over: they explain deletions that happened to
    /// the original, and the copy has no history yet.
    pub fn duplicated(&self) -> Scene {
        let mut copy = Scene {
            id: SceneId::new(),
            name: self.name.clone(),
            summary: self.summary.clone(),
            participants: self.participants.clone(),
            entry: self.entry.clone(),
            beats: self.beats.iter().map(Beat::duplicated).collect(),
            tombstones: Vec::new(),
        };

        let remap: Vec<(BeatId, BeatId)> =
            self.beats.iter().zip(&copy.beats).map(|(old, new)| (old.id, new.id)).collect();
        let lookup = |id: BeatId| remap.iter().find(|(old, _)| *old == id).map(|(_, new)| *new);

        for beat in &mut copy.beats {
            let destinations = beat
                .choices
                .iter_mut()
                .map(|c| &mut c.to)
                .chain(beat.outcomes.iter_mut().map(|o| &mut o.to));
            for destination in destinations {
                if let Destination::Beat(target) = destination
                    && let Some(new) = lookup(*target)
                {
                    *destination = Destination::Beat(new);
                }
            }
        }
        copy
    }

    /// Every dialogue slot in the scene, with the beat it belongs to.
    pub fn dialogue_slots(&self) -> impl Iterator<Item = (BeatId, &DialogueSlot)> {
        self.beats.iter().flat_map(|beat| beat.dialogue.iter().map(move |slot| (beat.id, slot)))
    }

    /// The scenes this one links out to, with the site responsible for each.
    ///
    /// Exposed separately because resolving them needs the rest of the project,
    /// which a caller holding one file does not have.
    pub fn scene_links(&self) -> impl Iterator<Item = (DestinationSite, SceneId)> {
        self.beats.iter().flat_map(Beat::destinations).filter_map(|(site, to)| match to {
            Destination::Scene(id) => Some((site, *id)),
            Destination::Beat(_) | Destination::End { .. } => None,
        })
    }
}
