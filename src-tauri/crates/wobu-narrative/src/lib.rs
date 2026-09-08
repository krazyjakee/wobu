//! The typed narrative source model: what a writer authored, before anything
//! has been compiled, generated or played.
//!
//! Scenes, beats, choices, outcomes, dialogue slots and variants, the stable
//! identities that hold them together, the content revisions that deliberately
//! do not, and a closed condition/effect language over declared state. It is
//! pure data: no IO, no async, no Tauri, no provider, no clock beyond the one
//! that stamps a deletion. It depends on `wobu-core` for exactly one thing — the
//! [`EntityId`] of a character who is already in the project — because a scene's
//! participants are the world's characters, not a second copy of them.
//!
//! ## What deliberately does not live here
//!
//! - **The compiler and the runtime** (#158, #159). Nothing in this crate
//!   evaluates a condition, resolves a branch, picks a variant or advances a
//!   cursor. [`Scene::destination_issues`] reads links; it does not decide
//!   whether they can be taken, and it makes no claim about reachability.
//! - **Persistence** (#153). No paths, no files, no index. The types are shaped
//!   to be easy to write down — one scene per document, ids that are stable
//!   across a rewrite, a version at the top — and that is the whole of this
//!   crate's involvement in storage.
//! - **Generation, providers and review** (#163–#166). [`Provenance`] and
//!   [`ContentLifecycle`] record what happened to a piece of text; nothing here
//!   makes it happen.
//! - **Canvas layout** (#185). There is no `x`, no `y`, no collapsed flag and no
//!   node ordering anywhere in this model. Where a beat sits is presentation
//!   metadata stored beside the source; a coordinate in here would land in a
//!   collaborator's merge, in the revision a translation is keyed to, and in the
//!   package the game ships, which is to say that moving a box would change the
//!   story. If a field on one of these structs starts to look like it wants a
//!   position, that is the bug #185 exists to prevent.
//!
//! ## The one distinction to read first
//!
//! An **id** names a slot and never changes. A **revision** names wording and
//! changes whenever the wording does. Renaming and reordering preserve ids;
//! duplication allocates new ones; deletion leaves a [`Tombstone`] so a
//! reference to what is gone can still be explained. See [`id`] for why getting
//! this backwards is unrecoverable.

pub mod diagnose;
pub mod error;
pub mod expr;
pub mod id;
pub mod lifecycle;
pub mod scene;
pub mod source;
pub mod state;
pub mod text;
pub mod world;

pub use diagnose::{Diagnostic, Problem, SceneCatalog, Site};
pub use error::{Error, Result, SourceLocation};
pub use expr::{
    Assignment, CompareOp, Comparison, Condition, Effect, HostCommand, Increment, Operand,
    TypeError,
};
pub use id::{
    BeatId, ChoiceId, DialogueSlotId, OutcomeId, Provenance, Revision, SceneId, TextAssetId,
    TextEntryId, VariantId,
};
pub use lifecycle::{ContentLifecycle, Freshness, GenerationPolicy, ReviewState};
pub use scene::{
    Beat, Choice, Destination, DestinationSite, DialogueSlot, EntityId, Intent, Outcome,
    Participant, Scene, Speaker, Text, Tombstone, TombstoneTarget, Variant,
};
pub use source::{
    SCENE_SCHEMA_VERSION, SOURCE_SCHEMA_VERSION, SceneDocument, StateDocument, WORLD_SCHEMA_VERSION,
};
pub use state::{Name, Owner, StateSchema, Value, VarType, VariableDecl};
pub use text::{
    Delivery, RepeatPolicy, SourceLink, TEXT_SCHEMA_VERSION, TextAsset, TextAssetDocument,
    TextEntry, TextKind, Trigger, Voice,
};

pub use world::{
    Belief, Fact, FutureRestriction, KnowledgeClaim, KnowledgeProvenance, NamedClassification,
    Quest, QuestTransition, Relationship, WorldDiagnostic, WorldDocument, WorldEvent,
};

mod source_paths;
pub use source_paths::SourcePathPart;

pub mod review;
