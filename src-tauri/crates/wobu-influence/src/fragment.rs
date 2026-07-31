//! What a layer actually contributes, and the rules that weight and route it.

use std::collections::BTreeMap;

use wobu_core::{FragmentTarget, Id, Layer};

use crate::stack::{Origin, ResolvedSource};

/// The body of a fragment: a span of prose, or a reference image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FragmentBody<'a> {
    /// One prose section, or one item of a list section, trimmed.
    ///
    /// Borrowed rather than owned because `prompt_compile` runs on every
    /// Inspector interaction (`docs/05-architecture.md`) and a five-layer stack
    /// is already a hundred or so of these. Copying every section on every drag
    /// of a slider is the one allocation pattern that would show up as a
    /// stutter, and the caller is holding the nodes anyway — see [`crate::World`].
    Text(&'a str),
    /// A reference image attached to the source node. The id and not the
    /// `wobu_core::Asset`, because assets are files behind the store and this
    /// crate does no IO; whatever builds a request resolves it there.
    Asset(Id),
}

/// One thing one layer contributes.
///
/// The fields are private, which is a departure from [`ResolvedSource`] next
/// door and is the entire point of the type. `layer` and the origin behind
/// `node_id` are not diagnostics to be filled in when convenient: the compiled
/// prompt is tinted by origin so that a user who does not like what came out can
/// see which upstream note to go and fix, and that feedback loop is what the app
/// is for (`docs/04-influence-engine.md`). A fragment that had lost track of
/// where it came from would be a span nobody can act on. The only constructor
/// takes the [`ResolvedSource`] it came from, so one cannot be built that does
/// not know.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fragment<'a> {
    layer: Layer,
    origin: Origin<'a>,
    section: &'static str,
    body: FragmentBody<'a>,
    weight: f32,
    target: FragmentTarget,
}

impl<'a> Fragment<'a> {
    /// Build a fragment attributed to the source it came from.
    pub fn new(
        source: &ResolvedSource<'a>,
        section: &'static str,
        body: FragmentBody<'a>,
        weight: f32,
        target: FragmentTarget,
    ) -> Fragment<'a> {
        Fragment { layer: source.layer, origin: source.origin, section, body, weight, target }
    }

    pub fn layer(self) -> Layer {
        self.layer
    }

    pub fn origin(self) -> Origin<'a> {
        self.origin
    }

    /// The node this came from, or `None` for the Shot layer, which has none.
    pub fn node_id(self) -> Option<Id> {
        match self.origin {
            Origin::Node(node) => Some(node.id),
            Origin::Shot(_) => None,
        }
    }

    /// What the layer card this belongs to is titled.
    pub fn source_name(self) -> &'a str {
        match self.origin {
            Origin::Node(node) => node.name.as_str(),
            Origin::Shot(label) => label,
        }
    }

    /// The description section key, or the reference's role for an image.
    ///
    /// `&'static str` and not `&'a str`: section keys are vocabulary from the
    /// kind and asset registries, never text out of a node's file. A key nothing
    /// declares is already dropped when the file is read
    /// (`Description::normalised_for`), so there is no path by which one could
    /// arrive from disk.
    pub fn section(self) -> &'static str {
        self.section
    }

    pub fn body(self) -> FragmentBody<'a> {
        self.body
    }

    pub fn text(self) -> Option<&'a str> {
        match self.body {
            FragmentBody::Text(text) => Some(text),
            FragmentBody::Asset(_) => None,
        }
    }

    pub fn asset_id(self) -> Option<Id> {
        match self.body {
            FragmentBody::Text(_) => None,
            FragmentBody::Asset(id) => Some(id),
        }
    }

    /// `link.weight × section_priority × user_slider`, already multiplied out.
    pub fn weight(self) -> f32 {
        self.weight
    }

    pub fn target(self) -> FragmentTarget {
        self.target
    }

    /// Whether this fragment may be put in front of a provider.
    ///
    /// Derived from [`target`](Self::target) and from nothing else, exactly as
    /// `AssetRole::is_conditioning` is in `wobu-core`: two lists of what is
    /// private would be one rename away from disagreeing, and the direction that
    /// disagreement fails in is somebody's mood board on a third party's
    /// servers. Everything that assembles a request — the budget (#43), the
    /// compiler, the provider adapters — filters on this rather than re-deriving
    /// the answer from the role.
    pub fn is_sendable(self) -> bool {
        !matches!(self.target, FragmentTarget::MoodboardOnly)
    }

    /// Whether this fragment carries any weight at all.
    ///
    /// A zero-weight fragment is kept rather than dropped. Zero is what a user
    /// gets by pulling a layer card's slider to the bottom or by setting a link
    /// weight to 0 in the world, and a card that listed three fragments before
    /// the drag and none after would read as the notes having been lost rather
    /// than turned down — in a panel whose whole job is showing where the prompt
    /// came from. It is the same reason the budget records `dropped` instead of
    /// truncating (`SnapshotFragment::dropped`). So extraction reports
    /// everything and this is the test a compiler uses to skip the silent ones.
    pub fn contributes(self) -> bool {
        self.weight > 0.0
    }
}

/// The sections that are not positive prompt material.
///
/// `never` is the only one — it is the negative prompt, and it is the one
/// section every kind is required to declare (`wobu-core`,
/// `every_kind_can_produce_a_negative_prompt`). Written as an exception table
/// rather than a `match` on the key so a test can sweep the kind registry and
/// pin the whole vocabulary longhand: a section added to a kind in `wobu-core`
/// then has to be argued about here instead of defaulting quietly into the
/// positive prompt, which is how this goes wrong without anything failing.
///
/// `palette` is deliberately *not* routed to [`FragmentTarget::Palette`]. That
/// target is the colour-conditioning channel and what reaches it is images
/// (`AssetRole::Palette`); the section is a list of `#rrggbb` strings, which is
/// prompt text. Handing words to an adapter that takes pictures would drop them
/// in silence, which is the failure `wobu-core`'s
/// `every_role_routes_to_the_target_the_engine_expects` guards in the other
/// direction.
const SECTION_TARGETS: &[(&str, FragmentTarget)] = &[("never", FragmentTarget::Negative)];

/// Where a description section's text is routed. Prose is positive unless the
/// table says otherwise.
pub fn section_target(section: &str) -> FragmentTarget {
    SECTION_TARGETS
        .iter()
        .find(|(key, _)| *key == section)
        .map(|(_, target)| *target)
        .unwrap_or(FragmentTarget::Prompt)
}

/// The `user_slider` term of `link.weight × section_priority × user_slider`.
///
/// One number per layer card, because that is what the control is: every card in
/// the Inspector carries a weight slider and a mute toggle, and reweighting
/// there affects one generation and never edits the world
/// (`docs/03-ui-layout.md`). Those controls do not exist yet (#47), so — as with
/// [`crate::Shot`] — this is the smallest thing that makes the weight formula
/// complete and honest without inventing the panel: a map from the node a card
/// is showing to where its slider sits. When the controls land they fill this in
/// and nothing here changes.
///
/// Keyed by node because a card is a source and a source is a node. The Shot
/// card's slider is `Shot::weight`, which resolution has already folded into
/// [`ResolvedSource::weight`], so every card in the panel has exactly one home
/// for its slider and no card can have its slider applied twice.
///
/// Mute is not here. It is the *other* control on the card, it removes a layer
/// rather than scaling it, and dropping muted layers is the compiler's first
/// filter step (`docs/04-influence-engine.md`). Expressing it as a slider at
/// zero would lose the difference between turned down and turned off, and
/// unmuting would restore the wrong number.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sliders {
    /// Ordered, not hashed, for the reason [`crate::World`] is: nothing in this
    /// crate may answer differently from one run to the next, and a map that
    /// iterates in a random order is a trap left for whoever iterates it next.
    by_node: BTreeMap<Id, f32>,
}

impl Sliders {
    /// Every card at 1.0 — what the Inspector shows before anyone touches
    /// anything, and what resolving for display rather than for a generation
    /// uses.
    pub fn neutral() -> Sliders {
        Sliders::default()
    }

    pub fn from_pairs(pairs: impl IntoIterator<Item = (Id, f32)>) -> Sliders {
        let mut sliders = Sliders::neutral();
        for (node, value) in pairs {
            sliders.set(node, value);
        }
        sliders
    }

    /// Clamped on the way in, not on the way out: 0.0–1.0 is a property of the
    /// control itself, the same range link and asset weights are held to in
    /// `wobu-core`. A value above it would let one card outweigh the preset's own
    /// section priorities, and the only symptom would be art that is wrong with
    /// nothing on screen to point at.
    pub fn set(&mut self, node: Id, value: f32) {
        self.by_node.insert(node, value.clamp(0.0, 1.0));
    }

    /// Where a node's slider sits. A card nobody has touched is at full weight.
    pub fn get(&self, node: Id) -> f32 {
        self.by_node.get(&node).copied().unwrap_or(1.0)
    }

    /// The multiplier for one layer card. 1.0 for the Shot card — see the type
    /// docs for where that card's slider lives instead.
    pub fn for_source(&self, source: &ResolvedSource<'_>) -> f32 {
        source.node_id().map_or(1.0, |id| self.get(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::Reached;

    /// A source with no node behind it, so these can be written without a world.
    fn shot_source() -> ResolvedSource<'static> {
        ResolvedSource {
            layer: Layer::Shot,
            origin: Origin::Shot("Character sheet · 3:4"),
            reached: Reached::Shot,
            distance: 0,
            weight: 1.0,
        }
    }

    fn fragment(weight: f32, target: FragmentTarget) -> Fragment<'static> {
        Fragment::new(&shot_source(), "framing", FragmentBody::Text("full body"), weight, target)
    }

    #[test]
    fn only_never_is_negative_and_everything_else_is_prompt_text() {
        // The routing rule in one place. A second negative section arriving by
        // accident would put "no modern firearms" into the positive prompt,
        // which is the exact instruction inversion the `never` list exists to
        // prevent, and nothing would throw.
        assert_eq!(section_target("never"), FragmentTarget::Negative);
        assert_eq!(section_target("silhouette"), FragmentTarget::Prompt);
        assert_eq!(section_target("palette"), FragmentTarget::Prompt);
        assert_eq!(section_target("a_section_nothing_declares"), FragmentTarget::Prompt);
    }

    #[test]
    fn a_fragment_is_sendable_unless_it_is_moodboard_only() {
        // The privacy property at the fragment layer. Stated over every target
        // rather than over the one that matters, so that a target added later is
        // classified here deliberately instead of inheriting `true`.
        let sendable = [
            (FragmentTarget::Prompt, true),
            (FragmentTarget::Negative, true),
            (FragmentTarget::StyleRef, true),
            (FragmentTarget::StructureRef, true),
            (FragmentTarget::Palette, true),
            (FragmentTarget::MoodboardOnly, false),
        ];
        for (target, expected) in sendable {
            assert_eq!(fragment(1.0, target).is_sendable(), expected, "{target:?}");
        }
    }

    #[test]
    fn a_zero_weight_fragment_still_exists_and_says_it_contributes_nothing() {
        // Turning a layer down to nothing must not make its rows vanish from the
        // panel that is meant to explain the prompt.
        let muted = fragment(0.0, FragmentTarget::Prompt);
        assert!(!muted.contributes());
        assert_eq!(muted.text(), Some("full body"));
        assert!(fragment(0.001, FragmentTarget::Prompt).contributes());
    }

    #[test]
    fn a_fragment_cannot_be_built_without_the_source_it_came_from() {
        // Not a behaviour test — a shape test. `Fragment::new` takes a
        // `ResolvedSource` and the fields are private, so attribution is not
        // something a call site can forget. If this ever compiles with a
        // struct literal instead, the guarantee is gone.
        let source = shot_source();
        let body = FragmentBody::Text("full body");
        let f = Fragment::new(&source, "framing", body, 1.0, FragmentTarget::Prompt);
        assert_eq!(f.layer(), Layer::Shot);
        assert_eq!(f.source_name(), "Character sheet · 3:4");
        assert_eq!(f.node_id(), None, "the shot is not a node");
        assert_eq!(f.asset_id(), None);
    }

    #[test]
    fn a_slider_outside_the_controls_range_cannot_amplify_a_card() {
        // Values reach this from a UI that does not exist yet, so the range is
        // enforced here rather than assumed of the caller.
        let id = Id::nil();
        let mut sliders = Sliders::neutral();
        assert_eq!(sliders.get(id), 1.0, "an untouched card is at full weight");

        sliders.set(id, 4.0);
        assert_eq!(sliders.get(id), 1.0);
        sliders.set(id, -1.0);
        assert_eq!(sliders.get(id), 0.0);
        assert_eq!(Sliders::from_pairs([(id, 0.5)]).get(id), 0.5);
        assert_eq!(Sliders::from_pairs([(id, 0.5)]).for_source(&shot_source()), 1.0);
    }
}
