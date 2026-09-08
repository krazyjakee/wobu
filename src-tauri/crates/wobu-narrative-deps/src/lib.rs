//! What each authored line was written against, and what has moved since
//! (#168).
//!
//! Freshness is the one dimension of [`ContentLifecycle`] that nothing was
//! allowed to set. [`Freshness`] says so in as many words: it is *derived*,
//! from source and context that lives outside `wobu-narrative`, and until this
//! crate existed there was nowhere for that derivation to live. This is that
//! place. It answers one question — *which lines does this edit affect, and
//! why* — and it answers it from canonical data alone.
//!
//! ## The two halves of a dependency, and why one of them is not enough
//!
//! A dependency set has **fields** and it has **queries**, and the second half
//! is the one that is easy to leave out and impossible to add later.
//!
//! - **Fields** are the addressable values a line actually read: this
//!   character's narrative voice, this beat's `must_convey`, this variant's
//!   `when`, this fact's record. Each is stored as an address and the content
//!   hash observed at that address, with `None` recorded for a lookup that
//!   found nothing. That `None` is load-bearing: it is what makes *deleting* a
//!   fact a change rather than an absence of one.
//! - **Queries** are the *candidate sets* a line was resolved against —
//!   "every restriction that binds this speaker", "every relationship from this
//!   speaker to a participant". A purely id-based reverse index gets this
//!   wrong, and gets it wrong silently: a relationship that did not exist when
//!   the line was generated has no id for the old request to have referenced,
//!   so nothing keyed on ids can notice it appearing. Recording the whole
//!   candidate set, and comparing sets rather than looking ids up, is what
//!   makes an *addition* visible. The candidate set is taken before conditions
//!   are evaluated, exactly as
//!   [`Query`](wobu_narrative_context::Query) does, because a record that is
//!   currently inactive is still a record whose arrival or departure changes
//!   what the resolver had to consider.
//!
//! ## Why the capture is scenario-independent
//!
//! [`wobu_narrative_context::resolve`] answers a different question: what one
//! frozen request read, in one chosen scenario, at one instant. That is the
//! right shape for a receipt and the wrong shape for an index, because a
//! project-wide index cannot privilege one preview's variable values without
//! becoming wrong for every other. So [`capture()`] takes the union over
//! scenarios: it records the candidate sets and the fields *any* scenario could
//! reach, and never evaluates a condition. It is a deliberate
//! over-approximation in exactly one direction — it can mark a line affected
//! that a particular scenario would not have cared about, and it can never miss
//! one. Addresses and hashes are shared with the resolver (see
//! [`wobu_narrative_context::content_hash`]) so the two vocabularies stay
//! comparable; `tests/resolver_vocabulary.rs` pins that.
//!
//! ## Why Flow layout cannot appear here
//!
//! [`Snapshot`] holds scenes, supporting text assets, the world document, the
//! declared state and character voices. There is no field on any of them that
//! a coordinate could be written into — `wobu-narrative`'s module documentation
//! is explicit that this is enforced by absence rather than by a filter — and
//! this crate reads no files at all, so it cannot reach `narrative/layout/`
//! even in principle. A layout-only edit therefore produces a byte-identical
//! [`DependencyIndex`], which `wobu-store`'s `flow_layout` evidence asserts
//! against a real project folder rather than taking on trust (#185).
//!
//! ## What this crate deliberately does not do
//!
//! It does not read files, write files, decide *what to rebuild* (#169), talk
//! to a provider, or touch a single word of authored content. Marking a line
//! out of date is a lifecycle flag and nothing else; the words, the revision,
//! the provenance, the policy and the approval are untouched, and the receipts
//! that recorded the old answer are retained rather than rewritten.

pub mod capture;
pub mod change;
pub mod index;

pub use capture::{Snapshot, capture, capture_scene, capture_text};
pub use change::{Affected, AffectedKind, Explanation, Reason, compare};
pub use index::{DependencyIndex, edge_keys};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use wobu_narrative::{
    BeatId, DialogueSlotId, SceneId, TextAssetId, TextEntryId, VariantId,
    lifecycle::ContentLifecycle, lifecycle::Freshness,
};

/// The shape of a [`DependencySet`] on disk and in the local index.
///
/// Separate from every other version in [`ToolVersions`] and also *inside* it,
/// which is not redundancy: a stored set from an older build has to be
/// rejected before it is compared, and a set captured by a newer build has to
/// invalidate results captured by an older one. The first needs a version to
/// read; the second needs it inside the fingerprint.
pub const DEPENDENCY_VERSION: u32 = 1;

/// Which authored line a dependency set belongs to.
///
/// Two variants rather than one widened struct, for the reason
/// [`TextTarget`](wobu_narrative::review::TextTarget) gives for being a sibling
/// of [`ReviewTarget`](wobu_narrative::review::ReviewTarget): a target that
/// could name a scene and an asset at once is a state no editor can produce and
/// every consumer would have to reject.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetRef {
    SceneLine { scene: SceneId, beat: BeatId, slot: DialogueSlotId, variant: VariantId },
    TextLine { asset: TextAssetId, entry: TextEntryId, slot: DialogueSlotId, variant: VariantId },
}

impl TargetRef {
    /// The wording identity, which is what every map here is keyed by.
    ///
    /// Keyed by [`VariantId`] rather than by the whole target for the reason
    /// [`CompileOptions`](wobu_narrative_compiler::CompileOptions) keys its two
    /// evidence maps that way: variant ids are globally unique ULIDs, so one
    /// key cannot name both a scene line and a bark, and a key that carried the
    /// container as well would need a different spelling for each kind.
    pub fn variant(&self) -> VariantId {
        match self {
            TargetRef::SceneLine { variant, .. } | TargetRef::TextLine { variant, .. } => *variant,
        }
    }

    /// The slot the wording lives in — the unit a recording or a translation is
    /// attached to.
    pub fn slot(&self) -> DialogueSlotId {
        match self {
            TargetRef::SceneLine { slot, .. } | TargetRef::TextLine { slot, .. } => *slot,
        }
    }

    /// A person-readable address, used as the third leg of an
    /// [`Explanation`] — "source field → context → *line*".
    pub fn line(&self) -> String {
        match self {
            TargetRef::SceneLine { scene, beat, slot, variant } => {
                format!("scene/{scene}/beat/{beat}/slot/{slot}/variant/{variant}")
            }
            TargetRef::TextLine { asset, entry, slot, variant } => {
                format!("text/{asset}/entry/{entry}/slot/{slot}/variant/{variant}")
            }
        }
    }
}

/// Every version that can change what a line *should* say without any authored
/// byte changing.
///
/// All of them are in the fingerprint, because each one is a real input. A new
/// prompt version asks the model for something different; a new context
/// resolver assembles different fragments from the same source; a new graph
/// version lowers the same source to a different runtime object; a new source
/// schema admits fields the previous build could not read. Leaving any of them
/// out produces the worst possible failure mode — content that looks current,
/// was produced by a toolchain that no longer exists, and would not be produced
/// again.
///
/// Held as named fields with a [`ToolVersions::components`] view rather than as
/// a map, so that adding a version is a compile error at every call site that
/// enumerates them rather than a silently missing entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolVersions {
    pub state_schema: u32,
    pub scene_schema: u32,
    pub world_schema: u32,
    pub text_schema: u32,
    pub context_resolver: u32,
    pub compiler_graph: u32,
    pub prompt: u32,
    pub output_schema: u32,
    pub review: u32,
    pub dependency: u32,
}

impl ToolVersions {
    /// What this build is. Read from the owning crates' own constants rather
    /// than restated here, so a crate that bumps its version cannot forget to
    /// bump the fingerprint that depends on it.
    pub fn current() -> ToolVersions {
        ToolVersions {
            state_schema: wobu_narrative::SOURCE_SCHEMA_VERSION,
            scene_schema: wobu_narrative::SCENE_SCHEMA_VERSION,
            world_schema: wobu_narrative::WORLD_SCHEMA_VERSION,
            text_schema: wobu_narrative::TEXT_SCHEMA_VERSION,
            context_resolver: wobu_narrative_context::CONTEXT_VERSION,
            compiler_graph: wobu_narrative_compiler::GRAPH_VERSION,
            prompt: wobu_narrative_generation::PROMPT_VERSION,
            output_schema: wobu_narrative_generation::OUTPUT_SCHEMA_VERSION,
            review: wobu_narrative::review::REVIEW_VERSION,
            dependency: DEPENDENCY_VERSION,
        }
    }

    /// Each version beside the name a person would recognise it by, in a fixed
    /// order. The order is the declaration order and is part of the
    /// fingerprint, which is why it is written out rather than derived from a
    /// map that a future field could reorder.
    pub fn components(&self) -> [(&'static str, u32); 10] {
        [
            ("state_schema", self.state_schema),
            ("scene_schema", self.scene_schema),
            ("world_schema", self.world_schema),
            ("text_schema", self.text_schema),
            ("context_resolver", self.context_resolver),
            ("compiler_graph", self.compiler_graph),
            ("prompt", self.prompt),
            ("output_schema", self.output_schema),
            ("review", self.review),
            ("dependency", self.dependency),
        ]
    }
}

impl Default for ToolVersions {
    fn default() -> ToolVersions {
        ToolVersions::current()
    }
}

/// Who produced a generated line, in the only terms that can change its words.
///
/// Present only for wording a provider wrote. Hand-written wording has no
/// producer and must not acquire one, because a writer's line does not become
/// stale when somebody switches the project's default model.
///
/// `settings` is a sorted map of canonical JSON rather than the provider's own
/// settings struct, which is the "normalise unordered inputs deterministically"
/// half of this: two settings objects that differ only in key order are the
/// same settings, and a fingerprint that disagreed would invalidate a project
/// every time serialization order changed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    pub provider: String,
    pub model: String,
    pub settings: BTreeMap<String, String>,
}

impl Producer {
    /// Build one from a provider's settings value of any shape.
    ///
    /// A non-object settings value is stored under the single key `value`
    /// rather than rejected: this is a fingerprint input, and refusing to
    /// fingerprint an unfamiliar settings shape would mean silently tracking
    /// nothing.
    pub fn of(provider: impl Into<String>, model: impl Into<String>, settings: &Json) -> Producer {
        let settings = match settings {
            Json::Object(map) => {
                map.iter().map(|(key, value)| (key.clone(), value.to_string())).collect()
            }
            other => BTreeMap::from([("value".to_string(), other.to_string())]),
        };
        Producer { provider: provider.into(), model: model.into(), settings }
    }
}

/// The pre-condition candidate set of one named query.
///
/// Mirrors [`wobu_narrative_context::Query`] field for field, including the
/// decision that an empty membership is meaningful rather than absent: "no
/// relationship from this speaker to anyone in this scene" is a fact about the
/// world that stops being true the moment somebody authors one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySet {
    /// Canonical JSON of the query's parameters, normalised through
    /// [`serde_json::Value`]'s object ordering so that two spellings of the
    /// same parameters hash alike.
    pub parameters: String,
    /// Candidate id → content hash of the whole record.
    pub members: BTreeMap<String, String>,
}

/// Everything one line was written against.
///
/// Ordered maps throughout, so that the serialization a fingerprint is taken
/// over is a function of the content and not of the order the capture happened
/// to walk in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencySet {
    pub version: u32,
    pub target: TargetRef,
    pub versions: ToolVersions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<Producer>,
    /// Source address → content hash, or `None` for an address that was looked
    /// up and found empty. The `None` is why deletion is a change.
    pub fields: BTreeMap<String, Option<String>>,
    /// Query name → its candidate set.
    pub queries: BTreeMap<String, QuerySet>,
}

impl DependencySet {
    /// The value a cache is keyed on.
    ///
    /// Domain-separated and length-prefixed for the reason
    /// [`Revision::of`](wobu_narrative::Revision::of) is: without a length
    /// prefix the boundary between two parts is guessable, and two different
    /// dependency sets can be made to collide by moving bytes across it.
    ///
    /// Taken over the canonical JSON of the whole set, which includes the
    /// target's own identity — so a wording moved to a new variant id is a new
    /// slot with no inherited freshness, which is the id/revision split applied
    /// to caching.
    pub fn fingerprint(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wobu-narrative/dependency/1\0");
        let bytes =
            serde_json::to_vec(self).expect("a dependency set contains only JSON-safe types");
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        hasher.finalize().to_hex().to_string()
    }

    /// The wording identity this set belongs to.
    pub fn variant(&self) -> VariantId {
        self.target.variant()
    }
}

/// Apply an invalidation to a lifecycle without touching anything else.
///
/// A free function rather than a method on [`ContentLifecycle`] because the
/// decision it encodes belongs to this crate: freshness is derived here, and
/// the lifecycle type's job is only to hold the answer. It exists at all so
/// that every caller that propagates invalidation into authored text goes
/// through one line of code that provably changes one field — the words, the
/// revision, the provenance, the generation policy and the review state are all
/// still whatever they were, so a locked line stays locked and an approval
/// stays recorded while ceasing to be release-ready.
pub fn mark(lifecycle: &mut ContentLifecycle) -> bool {
    if lifecycle.freshness == Freshness::OutOfDate {
        return false;
    }
    lifecycle.mark_out_of_date();
    true
}
