//! Source-level analysis: what is broken, and who is responsible for it.
//!
//! Be clear about what this is not. It is not the compiler (#158) and it does
//! not do reachability: nothing here evaluates a condition, decides whether a
//! branch can be taken, or claims a beat is unreachable. Those answers need the
//! whole project and a search over state, and getting them wrong in the
//! reassuring direction — "no problems found" — is worse than not offering
//! them.
//!
//! What it does is answer the questions that can be settled by reading one
//! scene against a set of declared variables and a list of scene ids: does this
//! destination name something that exists, does this comparison type-check, is
//! this speaker actually in the scene. Every answer is keyed to the element
//! responsible, so #186 can select that element and #189 can mark that node.

use std::collections::HashSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::expr::TypeError;
use crate::id::{BeatId, DialogueSlotId, Revision, SceneId, VariantId};
use crate::scene::{Destination, DestinationSite, EntityId, Scene, Tombstone, TombstoneTarget};
use crate::state::StateSchema;

/// The scene ids a project contains.
///
/// Handed in rather than looked up, because this crate has no store and no
/// filesystem — the boundary that keeps it testable and keeps #153's job
/// separate from this one.
#[derive(Debug, Clone, Default)]
pub struct SceneCatalog {
    /// `None` means "not known", not "empty".
    known: Option<HashSet<SceneId>>,
}

impl SceneCatalog {
    /// A catalog that declines to answer.
    ///
    /// For a caller holding one file and not the project around it — the Source
    /// editor with an unsaved buffer, a unit test, a CLI linting a single scene.
    /// Cross-scene links are reported as nothing rather than as broken, because
    /// "every link out of this scene is dangling" is a false alarm that would
    /// train people to ignore the list.
    pub fn unknown() -> SceneCatalog {
        SceneCatalog { known: None }
    }

    /// A catalog that knows the whole project.
    pub fn of(ids: impl IntoIterator<Item = SceneId>) -> SceneCatalog {
        SceneCatalog { known: Some(ids.into_iter().collect()) }
    }

    /// `Some(false)` only when the catalog is complete and the scene is not in
    /// it.
    fn resolves(&self, id: SceneId) -> Option<bool> {
        self.known.as_ref().map(|known| known.contains(&id))
    }
}

/// Where in a scene a diagnostic belongs.
///
/// Fine-grained on purpose: a diagnostic that can only say "somewhere in this
/// scene" cannot be turned into a selection, and #156 requires that a broken
/// reference links to the responsible field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Site {
    Scene,
    /// The scene's entry condition rather than the scene as a whole.
    Entry,
    Participant {
        entity: EntityId,
    },
    Destination(DestinationSite),
    /// An intent, by its position in the beat's list. Intents have no id
    /// because nothing refers to one: they are prose attached to a beat, and
    /// giving them stable identities would imply a permanence they do not have.
    Intent {
        beat: BeatId,
        index: usize,
    },
    DialogueSlot {
        beat: BeatId,
        slot: DialogueSlotId,
    },
    Variant {
        beat: BeatId,
        slot: DialogueSlotId,
        variant: VariantId,
    },
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Site::Scene => f.write_str("scene"),
            Site::Entry => f.write_str("scene entry condition"),
            Site::Participant { entity } => write!(f, "participant {entity}"),
            Site::Destination(DestinationSite::Choice { beat, choice }) => {
                write!(f, "beat {beat}, choice {choice}")
            }
            Site::Destination(DestinationSite::Outcome { beat, outcome }) => {
                write!(f, "beat {beat}, outcome {outcome}")
            }
            Site::Destination(DestinationSite::Beat(beat)) => write!(f, "beat {beat}"),
            Site::Intent { beat, index } => write!(f, "beat {beat}, intent {index}"),
            Site::DialogueSlot { beat, slot } => write!(f, "beat {beat}, slot {slot}"),
            Site::Variant { beat, slot, variant } => {
                write!(f, "beat {beat}, slot {slot}, variant {variant}")
            }
        }
    }
}

/// What is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Problem {
    #[error("this route is disconnected; choose a destination or an explicit end")]
    UnresolvedDestination,
    #[error("{field} references classification {id}, which is not defined in World")]
    UnknownClassification { field: &'static str, id: EntityId },
    #[error("this destination names beat {beat}, which is not in this scene")]
    DanglingBeat { beat: BeatId },

    #[error(
        "this destination names beat {beat}, `{label}`, which was deleted {deleted_at}{}",
        .reason.as_ref().map(|r| format!(" ({r})")).unwrap_or_default()
    )]
    DeletedBeat { beat: BeatId, label: String, deleted_at: DateTime<Utc>, reason: Option<String> },

    #[error("this destination names scene {scene}, which is not in this project")]
    UnknownScene { scene: SceneId },

    #[error(
        "this beat offers no choice and no outcome, so the story stops here without saying so. \
         Add a choice, an outcome, or an explicit end."
    )]
    NoDestination,

    #[error("this scene has no beats")]
    NoBeats,

    #[error(transparent)]
    Type(#[from] TypeError),

    #[error("{entity} speaks here but is not a participant in this scene")]
    NotAParticipant { entity: EntityId },

    #[error("this slot has no text yet")]
    MissingText,

    #[error(
        "the recorded revision {recorded} does not describe the text here, which hashes to \
         {computed}. Anything keyed to the recorded revision — a translation, a recording, an \
         approval — is about to stop matching."
    )]
    RevisionMismatch { recorded: Revision, computed: Revision },

    #[error("{noun} id {id} appears more than once in this scene")]
    DuplicateId { noun: &'static str, id: String },
}

impl Problem {
    /// Whether this is one of the destination problems, so a caller wanting only
    /// the canvas wiring can filter without re-running the analysis.
    pub fn is_destination(&self) -> bool {
        matches!(
            self,
            Problem::UnresolvedDestination
                | Problem::DanglingBeat { .. }
                | Problem::DeletedBeat { .. }
                | Problem::UnknownScene { .. }
                | Problem::NoDestination
        )
    }
}

/// One problem, and the element responsible for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub site: Site,
    pub problem: Problem,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.site, self.problem)
    }
}

impl Diagnostic {
    fn at(site: Site, problem: Problem) -> Diagnostic {
        Diagnostic { site, problem }
    }
}

impl Scene {
    /// Unresolved and dangling destinations, keyed by the choice or outcome
    /// responsible.
    ///
    /// This is the source-level question #186 and #189 need answered: which link
    /// on this canvas points at nothing. A beat that names a deleted beat is
    /// reported with the tombstone's label and date rather than as an unknown
    /// id, which is the whole reason deletions leave tombstones behind.
    ///
    /// Cross-scene links are only checked when `catalog` knows the project; see
    /// [`SceneCatalog::unknown`].
    pub fn destination_issues(&self, catalog: &SceneCatalog) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for beat in &self.beats {
            if !beat.has_destination() {
                out.push(Diagnostic::at(
                    Site::Destination(DestinationSite::Beat(beat.id)),
                    Problem::NoDestination,
                ));
            }
            for (site, destination) in beat.destinations() {
                let site = Site::Destination(site);
                match destination {
                    Destination::Unresolved {} => {
                        out.push(Diagnostic::at(site, Problem::UnresolvedDestination))
                    }
                    Destination::Beat(target) if !self.has_beat(*target) => {
                        out.push(Diagnostic::at(site, self.explain_missing_beat(*target)));
                    }
                    Destination::Scene(target) if catalog.resolves(*target) == Some(false) => {
                        out.push(Diagnostic::at(site, Problem::UnknownScene { scene: *target }));
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// The best explanation available for a destination that does not resolve.
    fn explain_missing_beat(&self, beat: BeatId) -> Problem {
        match self.beat_tombstone(beat) {
            Some(Tombstone { label, deleted_at, reason, .. }) => Problem::DeletedBeat {
                beat,
                label: label.clone(),
                deleted_at: *deleted_at,
                reason: reason.clone(),
            },
            None => Problem::DanglingBeat { beat },
        }
    }

    /// Everything [`Scene::destination_issues`] reports, plus reference,
    /// identity and type checks.
    ///
    /// A list rather than a `Result`, because a scene half-written is the normal
    /// state of a scene and refusing to open one until every branch is finished
    /// would make the tool unusable. Order is stable — scene, then beats in
    /// author order — so a list rendered from it does not jump between runs.
    pub fn diagnostics(&self, schema: &StateSchema, catalog: &SceneCatalog) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        if self.beats.is_empty() {
            out.push(Diagnostic::at(Site::Scene, Problem::NoBeats));
        }
        if let Some(entry) = &self.entry
            && let Err(err) = schema.check_condition(entry)
        {
            out.push(Diagnostic::at(Site::Entry, err.into()));
        }
        self.check_unique_ids(&mut out);

        let participants: HashSet<EntityId> = self.participants.iter().map(|p| p.entity).collect();

        for beat in &self.beats {
            for (index, intent) in beat.intents.iter().enumerate() {
                if let Some(entity) = intent.subject.entity()
                    && !participants.contains(&entity)
                {
                    out.push(Diagnostic::at(
                        Site::Intent { beat: beat.id, index },
                        Problem::NotAParticipant { entity },
                    ));
                }
            }

            for slot in &beat.dialogue {
                let site = Site::DialogueSlot { beat: beat.id, slot: slot.id };
                if let Some(entity) = slot.speaker.entity()
                    && !participants.contains(&entity)
                {
                    out.push(Diagnostic::at(site, Problem::NotAParticipant { entity }));
                }
                if slot.is_missing_text() {
                    out.push(Diagnostic::at(site, Problem::MissingText));
                }
                for variant in &slot.variants {
                    let site = Site::Variant { beat: beat.id, slot: slot.id, variant: variant.id };
                    if let Some(when) = &variant.when
                        && let Err(err) = schema.check_condition(when)
                    {
                        out.push(Diagnostic::at(site, err.into()));
                    }
                    if !variant.text.revision_matches() {
                        out.push(Diagnostic::at(
                            site,
                            Problem::RevisionMismatch {
                                recorded: variant.text.revision.clone(),
                                computed: variant.text.computed_revision(),
                            },
                        ));
                    }
                }
            }

            for choice in &beat.choices {
                let site =
                    Site::Destination(DestinationSite::Choice { beat: beat.id, choice: choice.id });
                if let Some(requires) = &choice.requires
                    && let Err(err) = schema.check_condition(requires)
                {
                    out.push(Diagnostic::at(site, err.into()));
                }
                for effect in &choice.effects {
                    if let Err(err) = schema.check_effect(effect) {
                        out.push(Diagnostic::at(site, err.into()));
                    }
                }
            }

            for outcome in &beat.outcomes {
                let site = Site::Destination(DestinationSite::Outcome {
                    beat: beat.id,
                    outcome: outcome.id,
                });
                if let Some(when) = &outcome.when
                    && let Err(err) = schema.check_condition(when)
                {
                    out.push(Diagnostic::at(site, err.into()));
                }
                for effect in &outcome.effects {
                    if let Err(err) = schema.check_effect(effect) {
                        out.push(Diagnostic::at(site, err.into()));
                    }
                }
            }
        }

        out.extend(self.destination_issues(catalog));
        out
    }

    /// Ids have to be unique within the scene, and a file that repeats one is
    /// almost always a copy-paste in the Source view.
    ///
    /// Worth checking rather than assuming, because everything downstream — the
    /// canvas selection, the locale pack, the recording script — indexes by
    /// these, and a silent duplicate means one row overwriting another.
    fn check_unique_ids(&self, out: &mut Vec<Diagnostic>) {
        let mut beats = HashSet::new();
        let mut slots = HashSet::new();
        let mut variants = HashSet::new();
        for beat in &self.beats {
            if !beats.insert(beat.id) {
                out.push(Diagnostic::at(
                    Site::Destination(DestinationSite::Beat(beat.id)),
                    Problem::DuplicateId { noun: BeatId::NOUN, id: beat.id.to_string() },
                ));
            }
            for slot in &beat.dialogue {
                if !slots.insert(slot.id) {
                    out.push(Diagnostic::at(
                        Site::DialogueSlot { beat: beat.id, slot: slot.id },
                        Problem::DuplicateId {
                            noun: DialogueSlotId::NOUN,
                            id: slot.id.to_string(),
                        },
                    ));
                }
                for variant in &slot.variants {
                    if !variants.insert(variant.id) {
                        out.push(Diagnostic::at(
                            Site::Variant { beat: beat.id, slot: slot.id, variant: variant.id },
                            Problem::DuplicateId {
                                noun: VariantId::NOUN,
                                id: variant.id.to_string(),
                            },
                        ));
                    }
                }
            }
        }
    }

    /// The tombstone explaining a production reference that no longer resolves —
    /// a locale row or a recording filed against a line that has been deleted
    /// (#178, #179).
    pub fn tombstone_for(&self, target: TombstoneTarget) -> Option<&Tombstone> {
        self.tombstones.iter().find(|t| t.target == target)
    }
}
