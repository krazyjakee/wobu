//! What resolution produces: an ordered list of sources, one per layer card.

use serde::Serialize;
use wobu_core::{Id, Layer, LinkRole, Node};

/// How a source got into the stack.
///
/// Carried through to the Inspector because a layer card that cannot say *why*
/// it is present is unarguable — the user sees a culture they did not expect and
/// has nowhere to go. It is also the only handle a test has on the shape of the
/// walk: an ancestry chain reached through the wrong edge still looks correct in
/// a list of names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reached {
    /// The node the user selected.
    Subject,
    /// A project singleton seeded into every stack: the Style Guide and the
    /// World Bible are roots whether or not anything links to them.
    Root,
    /// Followed an explicit link of this role.
    Link(LinkRole),
    /// Followed `parent_id`, the implicit link of weight 1.0.
    Parent,
    /// The shot controls, which are not part of the world.
    Shot,
}

/// Where a layer's text and images will come from.
///
/// Not simply `&Node`, because layer 7 has no node behind it. `SnapshotLayer`
/// already anticipates this with a nullable `node_id`; this is the same fact
/// stated in a form that cannot be forgotten at the call site.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin<'a> {
    Node(&'a Node),
    /// The Shot layer, labelled by whatever the caller is presenting as the
    /// current shot.
    Shot(&'a str),
}

/// The Shot layer's input.
///
/// Layer 7 comes from Inspector controls — output preset, aspect, model, seed —
/// that do not exist yet (#47). Rather than invent their shape here and be wrong
/// about it, the crate takes the layer as a caller-supplied label and weight:
/// enough for the layer to be real, be ordered last, and appear in the stack, and
/// nothing that a UI decision could invalidate. When the controls land they add
/// fields to this struct and nothing about the walk changes, because the shot is
/// not reachable from the graph and never participates in it.
///
/// `None` at the call site means the caller is resolving the stack for display
/// (`influence_resolve`) rather than for a generation, and there is no shot yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot<'a> {
    pub label: &'a str,
    pub weight: f32,
}

impl<'a> Shot<'a> {
    pub fn new(label: &'a str) -> Shot<'a> {
        Shot { label, weight: 1.0 }
    }
}

/// One collected source — one layer card in the Inspector.
///
/// The seam for #42 and #43: these are exactly the fields of a `SnapshotLayer`
/// minus `fragments`, which fragment extraction fills in, and `muted`, which is a
/// per-generation Inspector state that resolution has no business knowing about.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedSource<'a> {
    pub layer: Layer,
    pub origin: Origin<'a>,
    pub reached: Reached,
    /// Hops from whichever root reached this source. The subject and the two
    /// seeded singletons are 0. Higher is further out, which is why sources sort
    /// by *descending* distance within a layer.
    pub distance: u16,
    /// The product of the link weights along the path that first reached this
    /// source, with `parent_id` counting as 1.0. Fragment scoring multiplies this
    /// by section priority and the user's slider (#42); it is kept separate so
    /// the two can be told apart in the snapshot afterwards.
    pub weight: f32,
}

impl<'a> ResolvedSource<'a> {
    pub fn node(&self) -> Option<&'a Node> {
        match self.origin {
            Origin::Node(node) => Some(node),
            Origin::Shot(_) => None,
        }
    }

    pub fn node_id(&self) -> Option<Id> {
        self.node().map(|n| n.id)
    }

    /// What the layer card is titled.
    pub fn name(&self) -> &'a str {
        match self.origin {
            Origin::Node(node) => node.name.as_str(),
            Origin::Shot(label) => label,
        }
    }
}

/// The resolved stack, outermost source first and the subject last before the
/// shot. The order is the contract — see the crate docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStack<'a> {
    pub subject: Id,
    pub sources: Vec<ResolvedSource<'a>>,
}

impl<'a> ResolvedStack<'a> {
    pub fn sources(&self) -> &[ResolvedSource<'a>] {
        &self.sources
    }

    pub fn in_layer(&self, layer: Layer) -> impl Iterator<Item = &ResolvedSource<'a>> {
        self.sources.iter().filter(move |s| s.layer == layer)
    }

    /// The subject's own card, absent only when the subject was reached as an
    /// outer layer first — a project whose Style Guide is the selected node.
    pub fn subject_source(&self) -> Option<&ResolvedSource<'a>> {
        self.sources.iter().find(|s| s.reached == Reached::Subject)
    }

    pub fn contains(&self, id: Id) -> bool {
        self.sources.iter().any(|s| s.node_id() == Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shot_source_has_no_node_behind_it() {
        // #42 will ask every source for its node when it goes looking for
        // description sections. The shot has none, and must not be a panic.
        let source = ResolvedSource {
            layer: Layer::Shot,
            origin: Origin::Shot("Character sheet · 3:4"),
            reached: Reached::Shot,
            distance: 0,
            weight: 1.0,
        };
        assert_eq!(source.node_id(), None);
        assert_eq!(source.name(), "Character sheet · 3:4");
    }

    #[test]
    fn shot_controls_default_to_full_weight() {
        assert_eq!(Shot::new("Turnaround").weight, 1.0);
    }

    #[test]
    fn reached_serialises_for_the_layer_card() {
        assert_eq!(serde_json::to_string(&Reached::Parent).unwrap(), r#""parent""#);
        assert_eq!(
            serde_json::to_string(&Reached::Link(LinkRole::SpeciesOf)).unwrap(),
            r#"{"link":"species_of"}"#
        );
    }
}
