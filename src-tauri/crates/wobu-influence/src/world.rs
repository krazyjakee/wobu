//! The borrowed view of a world that the engine resolves against.

use std::collections::BTreeMap;

use wobu_core::{Id, Node, NodeKind};

/// Already-loaded nodes, indexed for the walk.
///
/// The engine takes this rather than a project handle because `prompt_compile`
/// runs on every Inspector interaction — every drag of a weight slider — and has
/// to stay sub-millisecond (`docs/05-architecture.md`). A type that *could*
/// reach the filesystem or the SQLite index would eventually be asked to, and
/// the panel would start stuttering on a network share for a reason invisible
/// from inside this crate. Borrowing nodes the caller has already loaded makes
/// that impossible rather than merely discouraged, and it is what keeps
/// `wobu-influence` independent of `wobu-store`.
///
/// Building one is cheap enough to do per call, but it is worth keeping for the
/// lifetime of a selection: the singleton roots are found once, here.
pub struct World<'a> {
    /// Ordered, not hashed. Resolution must answer identically on every run and
    /// every machine, and a `HashMap` scan for the Style Guide would return
    /// whichever entry the process's random hash seed happened to place first —
    /// so a project that had somehow acquired two would render differently from
    /// launch to launch with nothing in the world to explain it. Ordered storage
    /// removes that failure mode instead of documenting it.
    nodes: BTreeMap<Id, &'a Node>,
    style_guide: Option<&'a Node>,
    world_bible: Option<&'a Node>,
}

impl<'a> World<'a> {
    pub fn new<I>(nodes: I) -> World<'a>
    where
        I: IntoIterator<Item = &'a Node>,
    {
        let mut map: BTreeMap<Id, &'a Node> = BTreeMap::new();
        for node in nodes {
            // The store keys a node's file by its id, so two entries under one
            // id can only mean the caller passed the same node twice.
            map.entry(node.id).or_insert(node);
        }
        // Found here rather than per resolve: these two are the roots of every
        // stack, and finding them is a scan of the whole world.
        let style_guide = lowest_of_kind(&map, NodeKind::StyleGuide);
        let world_bible = lowest_of_kind(&map, NodeKind::WorldBible);
        World { nodes: map, style_guide, world_bible }
    }

    pub fn get(&self, id: Id) -> Option<&'a Node> {
        self.nodes.get(&id).copied()
    }

    /// The project's Style Guide — layer 1 of every stack. `None` for a project
    /// that has not created one yet, which is a normal state on day one.
    pub fn style_guide(&self) -> Option<&'a Node> {
        self.style_guide
    }

    /// The project's World Bible — layer 2 of every stack.
    pub fn world_bible(&self) -> Option<&'a Node> {
        self.world_bible
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The lowest id of a kind, or `None`.
///
/// Both kinds this is used for are `singleton` in the registry, so a second one
/// can only arrive by hand-editing or by two collaborators creating one at once
/// on a share. Lowest ULID means oldest, which is the one the project has been
/// rendering with — and, more to the point, it is an answer that does not depend
/// on which order the caller happened to hand us its nodes.
fn lowest_of_kind<'a>(nodes: &BTreeMap<Id, &'a Node>, kind: NodeKind) -> Option<&'a Node> {
    nodes.values().find(|n| n.kind == kind).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: NodeKind, name: &str) -> Node {
        Node::new(kind, name).unwrap()
    }

    #[test]
    fn the_roots_of_every_stack_are_found_by_kind() {
        let style = node(NodeKind::StyleGuide, "Ashfall House Style");
        let bible = node(NodeKind::WorldBible, "Ashfall");
        let kael = node(NodeKind::Character, "Kael Vantris");
        let world = World::new([&kael, &style, &bible]);

        assert_eq!(world.style_guide().map(|n| n.id), Some(style.id));
        assert_eq!(world.world_bible().map(|n| n.id), Some(bible.id));
        assert_eq!(world.len(), 3);
    }

    #[test]
    fn a_project_without_the_singletons_still_builds_a_view() {
        // Every project is in this state between `project_create` and the user
        // writing anything, and the Inspector is on screen for all of it.
        let kael = node(NodeKind::Character, "Kael Vantris");
        let world = World::new([&kael]);
        assert!(world.style_guide().is_none());
        assert!(world.world_bible().is_none());
    }

    #[test]
    fn a_duplicated_style_guide_resolves_to_the_older_one_whatever_the_load_order() {
        // Two collaborators on a share can each create the singleton. The pick
        // must not depend on which order the store yielded them, or the same
        // project renders differently on two machines.
        let a = node(NodeKind::StyleGuide, "House Style");
        let b = node(NodeKind::StyleGuide, "House Style 2");
        let oldest = if a.id < b.id { a.id } else { b.id };

        assert_eq!(World::new([&a, &b]).style_guide().map(|n| n.id), Some(oldest));
        assert_eq!(World::new([&b, &a]).style_guide().map(|n| n.id), Some(oldest));
    }

    #[test]
    fn an_unknown_id_looks_up_to_nothing() {
        let kael = node(NodeKind::Character, "Kael Vantris");
        let world = World::new([&kael]);
        assert!(world.get(wobu_core::new_id()).is_none());
        assert_eq!(world.get(kael.id).map(|n| n.name.as_str()), Some("Kael Vantris"));
    }
}
