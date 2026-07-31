//! The walk.

use std::collections::{BTreeSet, VecDeque};

use wobu_core::{Id, Layer, LinkRole, Node};

use crate::stack::{Origin, Reached, ResolvedSource, ResolvedStack, Shot};
use crate::world::World;

/// A node that has been reached and is waiting to be expanded.
struct Pending<'a> {
    node: &'a Node,
    layer: Layer,
    reached: Reached,
    weight: f32,
    distance: u16,
}

/// Resolve the influence stack for `subject`.
///
/// Returns `None` when `subject` is not in the view. That is a bug at the call
/// site — the caller loaded the wrong set of nodes — not a world a user can
/// create, so it is worth distinguishing from the many legitimately thin stacks
/// (no Style Guide yet, no links yet) which resolve to a short list rather than
/// an error.
pub fn resolve<'a>(
    world: &World<'a>,
    subject: Id,
    shot: Option<Shot<'a>>,
) -> Option<ResolvedStack<'a>> {
    let subject_node = world.get(subject)?;

    let mut visited: BTreeSet<Id> = BTreeSet::new();
    let mut queue: VecDeque<Pending<'a>> = VecDeque::new();
    let mut sources: Vec<ResolvedSource<'a>> = Vec::new();

    // The roots, in this order. The subject goes in first so that a project
    // whose selected node *is* the Style Guide resolves it as the subject: it is
    // the thing being drawn, and first-visit-wins then leaves layer 1 empty
    // rather than leaving the stack with nothing to draw.
    //
    // The singletons follow, ahead of anything the walk finds, so that a
    // `styled_by` link that happens to reach the Style Guide at depth cannot
    // demote it out of layer 1.
    enqueue(&mut queue, &mut visited, Pending {
        node: subject_node,
        layer: Layer::Subject,
        reached: Reached::Subject,
        weight: 1.0,
        distance: 0,
    });
    for (root, layer) in
        [(world.style_guide(), Layer::Style), (world.world_bible(), Layer::World)]
    {
        if let Some(node) = root {
            enqueue(&mut queue, &mut visited, Pending {
                node,
                layer,
                reached: Reached::Root,
                weight: 1.0,
                distance: 0,
            });
        }
    }

    while let Some(current) = queue.pop_front() {
        sources.push(ResolvedSource {
            layer: current.layer,
            origin: Origin::Node(current.node),
            reached: current.reached,
            distance: current.distance,
            weight: current.weight,
        });

        // A lateral link is a nod to a sibling entity, not an inheritance edge,
        // so it contributes its own source and stops there. Expanding one would
        // pull that sibling's entire ancestry, culture and place chain into this
        // subject's stack, and two related characters would compile to nearly
        // the same prompt — the exact convergence the layering exists to prevent.
        if current.reached == Reached::Link(LinkRole::RelatedTo) {
            continue;
        }

        // `parent_id` is an implicit link of weight 1.0
        // (`docs/02-data-model.md`), and because nesting is always within a
        // kind, the parent contributes at the *same* layer as the child: a
        // District's City is still Place. Walked ahead of the explicit links
        // because nesting is structural and the user cannot reorder it, which
        // makes it the stabler of the two tie-breaks.
        if let Some(parent) = current.node.parent_id.and_then(|id| world.get(id)) {
            enqueue(&mut queue, &mut visited, Pending {
                node: parent,
                layer: current.layer,
                reached: Reached::Parent,
                weight: current.weight,
                distance: current.distance + 1,
            });
        }

        for link in &current.node.links {
            // `enabled` is the world's own off switch, stored in frontmatter.
            // Muting a layer for one generation is the Inspector's job and never
            // reaches this crate.
            if !link.enabled {
                continue;
            }
            // A link whose target is not in the view points at a deleted node.
            // The store reports those; refusing to resolve the subject over one
            // would take the Inspector down for a dangling edge the user cannot
            // even see from here.
            let Some(target) = world.get(link.to_id) else { continue };
            enqueue(&mut queue, &mut visited, Pending {
                node: target,
                layer: link.role.layer(),
                reached: Reached::Link(link.role),
                // Weights multiply along the chain: a character held loosely to
                // its culture is held no more tightly to that culture's parent.
                // Taking only the last edge's weight would let a 0.2 link be
                // undone by a 1.0 one further out.
                weight: current.weight * link.weight.clamp(0.0, 1.0),
                distance: current.distance + 1,
            });
        }
    }

    if let Some(shot) = shot {
        sources.push(ResolvedSource {
            layer: Layer::Shot,
            origin: Origin::Shot(shot.label),
            reached: Reached::Shot,
            distance: 0,
            weight: shot.weight.clamp(0.0, 1.0),
        });
    }

    sources.sort_by(|a, b| {
        a.layer
            .order()
            .cmp(&b.layer.order())
            // Outermost first within a layer: Region, then City, then District.
            // Reversed, the prompt would read specific-to-general and the
            // recency bias every text encoder has would land on the region
            // instead of the subject — art that is subtly wrong forever and
            // never throws.
            .then(b.distance.cmp(&a.distance))
        // Sources that tie on both keep discovery order, because `sort_by` is a
        // stable sort. Discovery order is the breadth-first walk over `links` as
        // written in the file, so a node with two species reads in the order the
        // user listed them.
    });

    Some(ResolvedStack { subject, sources })
}

/// Admit a reached node to the walk, or drop it because something already
/// claimed it.
///
/// First visit wins. That is what breaks cycles — a world with a link loop is
/// bounded by the number of nodes in it rather than by a hop counter — and it is
/// also what decides the layer of a node reachable by more than one route: the
/// nearest path wins, with the three roots enqueued before any of them.
fn enqueue<'a>(
    queue: &mut VecDeque<Pending<'a>>,
    visited: &mut BTreeSet<Id>,
    pending: Pending<'a>,
) {
    if visited.insert(pending.node.id) {
        queue.push_back(pending);
    }
}
