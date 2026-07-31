//! Snapshot tests over fixture worlds.
//!
//! A stack-resolution regression is invisible: nothing throws, nothing looks
//! broken, and the art comes out subtly wrong for months. So every test here
//! writes out the whole expected stack — layer and source name, in order — rather
//! than asserting a property of it. A diff on one of these is meant to be read
//! and argued with, which is also why the snapshots are longhand `Vec`s in the
//! test rather than opaque generated files.
//!
//! The main world mirrors `examples/Ashfall.wobu`, so the snapshots can be
//! checked against the worked example in `docs/guide/influence.html` by eye.

use wobu_core::{Layer, Link, LinkRole, Node, NodeKind};
use wobu_influence::{Reached, ResolvedStack, Shot, World, resolve};

fn node(kind: NodeKind, name: &str) -> Node {
    Node::new(kind, name).expect("fixture names are sluggable")
}

fn link(from: &mut Node, to: &Node, role: LinkRole) {
    from.links.push(Link::new(to.id, role));
}

fn weighted_link(from: &mut Node, to: &Node, role: LinkRole, weight: f32) {
    from.links.push(Link { weight, ..Link::new(to.id, role) });
}

/// The snapshot: what the Inspector would show, top to bottom.
fn snapshot<'a>(stack: &ResolvedStack<'a>) -> Vec<(Layer, &'a str)> {
    stack.sources().iter().map(|s| (s.layer, s.name())).collect()
}

/// `examples/Ashfall.wobu`, in memory. Kept in the same shape as the file
/// fixture — including the `related_to` links that point at the World Bible,
/// which is a real thing hand-written worlds do and a trap for the layer
/// assignment.
struct Ashfall {
    style: Node,
    bible: Node,
    vashk: Node,
    ember_guild: Node,
    ember_coast: Node,
    cinder_bay: Node,
    kael: Node,
    lantern: Node,
}

impl Ashfall {
    fn new() -> Ashfall {
        let style = node(NodeKind::StyleGuide, "Ashfall House Style");

        let mut bible = node(NodeKind::WorldBible, "Ashfall");
        link(&mut bible, &style, LinkRole::StyledBy);

        let mut vashk = node(NodeKind::Species, "Vashk");
        link(&mut vashk, &style, LinkRole::StyledBy);
        link(&mut vashk, &bible, LinkRole::RelatedTo);

        let mut ember_guild = node(NodeKind::Culture, "Ember Guild");
        link(&mut ember_guild, &style, LinkRole::StyledBy);
        link(&mut ember_guild, &bible, LinkRole::RelatedTo);

        let mut ember_coast = node(NodeKind::Setting, "The Ember Coast");
        link(&mut ember_coast, &style, LinkRole::StyledBy);

        let mut cinder_bay = node(NodeKind::Setting, "Cinder Bay");
        cinder_bay.parent_id = Some(ember_coast.id);
        link(&mut cinder_bay, &style, LinkRole::StyledBy);

        let mut kael = node(NodeKind::Character, "Kael Vantris");
        link(&mut kael, &vashk, LinkRole::SpeciesOf);
        link(&mut kael, &ember_guild, LinkRole::MemberOf);
        link(&mut kael, &cinder_bay, LinkRole::LocatedIn);

        let mut lantern = node(NodeKind::Prop, "Ashglass Lantern");
        link(&mut lantern, &ember_guild, LinkRole::MemberOf);
        link(&mut lantern, &cinder_bay, LinkRole::LocatedIn);
        link(&mut lantern, &kael, LinkRole::RelatedTo);

        Ashfall { style, bible, vashk, ember_guild, ember_coast, cinder_bay, kael, lantern }
    }

    fn nodes(&self) -> Vec<&Node> {
        vec![
            &self.style,
            &self.bible,
            &self.vashk,
            &self.ember_guild,
            &self.ember_coast,
            &self.cinder_bay,
            &self.kael,
            &self.lantern,
        ]
    }

    fn world(&self) -> World<'_> {
        World::new(self.nodes())
    }
}

#[test]
fn a_character_resolves_in_the_documented_layer_order() {
    // The snapshot the whole engine is judged against: the seven layers of
    // docs/04, in order, for the worked example in the guide.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Character sheet · 3:4"))).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Style, "Ashfall House Style"),
        (Layer::World, "Ashfall"),
        (Layer::Ancestry, "Vashk"),
        (Layer::Culture, "Ember Guild"),
        (Layer::Place, "The Ember Coast"),
        (Layer::Place, "Cinder Bay"),
        (Layer::Subject, "Kael Vantris"),
        (Layer::Shot, "Character sheet · 3:4"),
    ]);
}

#[test]
fn the_place_chain_runs_region_to_district() {
    // Region → City → District, outermost first. The one ordering in the spec
    // stated as a literal sequence, and the one a reversed sort would invert
    // without any other symptom.
    let region = node(NodeKind::Setting, "The Ember Coast");
    let mut city = node(NodeKind::Setting, "Cinder Bay");
    let mut district = node(NodeKind::Setting, "The Pilings");
    city.parent_id = Some(region.id);
    district.parent_id = Some(city.id);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, &district, LinkRole::LocatedIn);

    // Deliberately loaded innermost-first, so a snapshot that came out in load
    // order rather than stack order would look right for the wrong reason.
    let nodes = vec![&district, &city, &region, &kael];
    let world = World::new(nodes);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Place, "The Ember Coast"),
        (Layer::Place, "Cinder Bay"),
        (Layer::Place, "The Pilings"),
        (Layer::Subject, "Kael Vantris"),
    ]);
    // Nesting is the implicit link, and it must be recorded as such: the
    // Inspector shows why a layer is present, and "located in" would be a lie
    // for two of these three.
    let reached: Vec<_> = stack.in_layer(Layer::Place).map(|s| s.reached).collect();
    assert_eq!(reached, vec![
        Reached::Parent,
        Reached::Parent,
        Reached::Link(LinkRole::LocatedIn)
    ]);

    // Region and city are nested, not weighted: `parent_id` is weight 1.0.
    assert!(stack.in_layer(Layer::Place).all(|s| s.weight == 1.0));
}

#[test]
fn the_ancestry_chain_runs_outermost_first() {
    // A sub-species reached through `species_of` and its parent species reached
    // through nesting have to interleave into one outermost-first chain, because
    // the body plan has to be established before the deviations from it.
    let ancestor = node(NodeKind::Species, "Vashk");
    let mut subspecies = node(NodeKind::Species, "Ashline Vashk");
    subspecies.parent_id = Some(ancestor.id);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, &subspecies, LinkRole::SpeciesOf);

    let world = World::new(vec![&ancestor, &subspecies, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Ancestry, "Vashk"),
        (Layer::Ancestry, "Ashline Vashk"),
        (Layer::Subject, "Kael Vantris"),
    ]);
}

#[test]
fn two_links_of_one_role_keep_the_order_they_are_written_in() {
    // The tie-break when neither layer nor distance separates two sources. It has
    // to be the user's own ordering: anything else and reordering a node's
    // Relations tab would silently change the compiled prompt.
    let first = node(NodeKind::Species, "Vashk");
    let second = node(NodeKind::Species, "Sunborn");
    let mut halfblood = node(NodeKind::Character, "Sister Oru");
    link(&mut halfblood, &first, LinkRole::SpeciesOf);
    link(&mut halfblood, &second, LinkRole::SpeciesOf);

    let world = World::new(vec![&first, &second, &halfblood]);
    let stack = resolve(&world, halfblood.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Ancestry, "Vashk"),
        (Layer::Ancestry, "Sunborn"),
        (Layer::Subject, "Sister Oru"),
    ]);
}

#[test]
fn resolution_does_not_depend_on_the_order_the_nodes_were_loaded() {
    // The "works on my machine" bug this crate is most exposed to: the caller
    // holds nodes in a HashMap, hands them over in whatever order that iterates
    // in, and the stack comes out differently on the next launch. Every
    // permutation of the same world must produce byte-identical output.
    let ashfall = Ashfall::new();
    let baseline = {
        let world = ashfall.world();
        let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Turnaround"))).unwrap();
        format!("{:?}", snapshot(&stack))
    };

    let mut nodes = ashfall.nodes();
    for rotation in 0..nodes.len() {
        nodes.rotate_left(1);
        let world = World::new(nodes.clone());
        let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Turnaround"))).unwrap();
        assert_eq!(format!("{:?}", snapshot(&stack)), baseline, "rotation {rotation}");
    }

    nodes.reverse();
    let world = World::new(nodes);
    let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Turnaround"))).unwrap();
    assert_eq!(format!("{:?}", snapshot(&stack)), baseline, "reversed");
}

#[test]
fn a_link_cycle_terminates_and_the_first_visit_wins() {
    // Nothing stops a user pointing two cultures at each other, and a walk that
    // hangs here would hang the whole Inspector.
    let mut inner = node(NodeKind::Culture, "Ember Guild");
    let mut outer = node(NodeKind::Culture, "Deepwardens");
    link(&mut inner, &outer, LinkRole::MemberOf);
    link(&mut outer, &inner, LinkRole::MemberOf);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, &inner, LinkRole::MemberOf);

    let world = World::new(vec![&inner, &outer, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Culture, "Deepwardens"),
        (Layer::Culture, "Ember Guild"),
        (Layer::Subject, "Kael Vantris"),
    ]);
}

#[test]
fn a_parent_ring_terminates() {
    // `validate_parent` refuses to create one, but a hand-edited file or a
    // half-applied sync on a share can still put one on disk, and the engine
    // reads what is there rather than what was allowed.
    let mut a = node(NodeKind::Setting, "Cinder Bay");
    let mut b = node(NodeKind::Setting, "The Ember Coast");
    a.parent_id = Some(b.id);
    b.parent_id = Some(a.id);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, &a, LinkRole::LocatedIn);

    let world = World::new(vec![&a, &b, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Place, "The Ember Coast"),
        (Layer::Place, "Cinder Bay"),
        (Layer::Subject, "Kael Vantris"),
    ]);
}

#[test]
fn a_node_reachable_by_two_routes_appears_once() {
    // The diamond: the lantern is both of the guild and in the bay, and the
    // guild is in the bay too. A second card for Cinder Bay would double its
    // fragments in the compiled prompt and quietly double its weight.
    let bay = node(NodeKind::Setting, "Cinder Bay");
    let mut guild = node(NodeKind::Culture, "Ember Guild");
    link(&mut guild, &bay, LinkRole::LocatedIn);

    let mut lantern = node(NodeKind::Prop, "Ashglass Lantern");
    link(&mut lantern, &guild, LinkRole::MemberOf);
    link(&mut lantern, &bay, LinkRole::LocatedIn);

    let world = World::new(vec![&bay, &guild, &lantern]);
    let stack = resolve(&world, lantern.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Culture, "Ember Guild"),
        (Layer::Place, "Cinder Bay"),
        (Layer::Subject, "Ashglass Lantern"),
    ]);
    // Reached at distance 1 by the direct link, not at distance 2 through the
    // guild — nearest wins, so the direct edge's weight is the one that counts.
    assert_eq!(stack.in_layer(Layer::Place).next().unwrap().distance, 1);
}

#[test]
fn a_lateral_link_joins_the_subject_layer_without_dragging_its_own_stack() {
    // `related_to` has no layer of its own, so it resolves at the subject's
    // level. It must not be walked through: Kael's species, culture and city
    // would then be the lantern's too, and a prop would compile to a prompt
    // describing a person.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.lantern.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Style, "Ashfall House Style"),
        (Layer::World, "Ashfall"),
        (Layer::Culture, "Ember Guild"),
        (Layer::Place, "The Ember Coast"),
        (Layer::Place, "Cinder Bay"),
        (Layer::Subject, "Kael Vantris"),
        (Layer::Subject, "Ashglass Lantern"),
    ]);
    // Kael came in laterally, and Kael's own species is not in the stack.
    assert!(!stack.contains(ashfall.vashk.id));
    // The subject is still the last thing before the shot, whatever else shares
    // its layer — the compiler relies on that for recency.
    assert_eq!(stack.sources().last().unwrap().reached, Reached::Subject);
}

#[test]
fn a_styled_by_link_cannot_demote_the_style_guide_out_of_layer_one() {
    // Everything in the Ashfall fixture points at the style guide, most of it
    // several hops out. If the walk claimed it at the distance it was found, the
    // Style layer would move around the stack depending on which node was
    // selected.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    for subject in [ashfall.kael.id, ashfall.cinder_bay.id, ashfall.ember_guild.id] {
        let stack = resolve(&world, subject, None).unwrap();
        assert_eq!(snapshot(&stack)[0], (Layer::Style, "Ashfall House Style"));
    }
}

#[test]
fn the_style_guide_resolves_as_its_own_subject() {
    // Selecting the Style Guide is an ordinary thing to do — it is pinned at the
    // top of the navigator. It is the thing being drawn, so it belongs in the
    // Subject layer, and first-visit-wins then leaves layer 1 empty rather than
    // listing it twice.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.style.id, Some(Shot::new("Material study"))).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::World, "Ashfall"),
        (Layer::Subject, "Ashfall House Style"),
        (Layer::Shot, "Material study"),
    ]);
    assert_eq!(stack.subject_source().unwrap().name(), "Ashfall House Style");
}

#[test]
fn a_world_with_no_style_guide_or_world_bible_still_resolves() {
    // Every project is in this state between creation and the user writing
    // anything, and the Inspector is on screen for all of it.
    let kael = node(NodeKind::Character, "Kael Vantris");
    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![(Layer::Subject, "Kael Vantris")]);
}

#[test]
fn a_subject_that_is_not_in_the_view_resolves_to_nothing() {
    // A caller bug rather than a world a user can create, and worth telling
    // apart from the many legitimately thin stacks above.
    let kael = node(NodeKind::Character, "Kael Vantris");
    let world = World::new([&kael]);
    assert!(resolve(&world, wobu_core::new_id(), None).is_none());
}

#[test]
fn a_link_to_a_deleted_node_is_stepped_over() {
    // Deleting a node leaves dangling edges in everything that referenced it.
    // The store surfaces those; the Inspector must still render the subject.
    let guild = node(NodeKind::Culture, "Ember Guild");
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, &guild, LinkRole::MemberOf);
    kael.links.push(Link::new(wobu_core::new_id(), LinkRole::SpeciesOf));

    let world = World::new(vec![&guild, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Culture, "Ember Guild"),
        (Layer::Subject, "Kael Vantris"),
    ]);
}

#[test]
fn a_disabled_link_is_not_walked() {
    // `enabled: false` is the world's own off switch. A disabled link that still
    // contributed would make the toggle look broken in the Relations tab.
    let bay = node(NodeKind::Setting, "Cinder Bay");
    let mut guild = node(NodeKind::Culture, "Ember Guild");
    link(&mut guild, &bay, LinkRole::LocatedIn);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link { enabled: false, ..Link::new(guild.id, LinkRole::MemberOf) });

    let world = World::new(vec![&bay, &guild, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![(Layer::Subject, "Kael Vantris")]);
}

#[test]
fn a_disabled_link_does_not_block_another_route_to_the_same_node() {
    // Turning off "member of the guild" must not also remove the city the guild
    // happens to sit in, if the subject is in that city in its own right.
    let bay = node(NodeKind::Setting, "Cinder Bay");
    let mut guild = node(NodeKind::Culture, "Ember Guild");
    link(&mut guild, &bay, LinkRole::LocatedIn);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link { enabled: false, ..Link::new(guild.id, LinkRole::MemberOf) });
    link(&mut kael, &bay, LinkRole::LocatedIn);

    let world = World::new(vec![&bay, &guild, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    assert_eq!(snapshot(&stack), vec![
        (Layer::Place, "Cinder Bay"),
        (Layer::Subject, "Kael Vantris"),
    ]);
}

#[test]
fn weights_multiply_along_the_chain_and_nesting_costs_nothing() {
    // A character held loosely to its culture is held no more tightly to that
    // culture's parent. Nesting is the implicit link of weight 1.0, so it must
    // pass the running weight through untouched rather than reset it.
    let outer = node(NodeKind::Culture, "Deepwardens");
    let mut guild = node(NodeKind::Culture, "Ember Guild");
    let region = node(NodeKind::Setting, "The Ember Coast");
    let mut bay = node(NodeKind::Setting, "Cinder Bay");
    bay.parent_id = Some(region.id);
    weighted_link(&mut guild, &outer, LinkRole::MemberOf, 0.5);

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    weighted_link(&mut kael, &guild, LinkRole::MemberOf, 0.4);
    weighted_link(&mut kael, &bay, LinkRole::LocatedIn, 0.5);

    let world = World::new(vec![&outer, &guild, &region, &bay, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();

    let weights: Vec<_> = stack.sources().iter().map(|s| (s.name(), s.weight)).collect();
    assert_eq!(weights, vec![
        ("Deepwardens", 0.2),
        ("Ember Guild", 0.4),
        ("The Ember Coast", 0.5),
        ("Cinder Bay", 0.5),
        ("Kael Vantris", 1.0),
    ]);
}

#[test]
fn a_hand_edited_weight_outside_the_range_cannot_amplify_a_chain() {
    // Weights come off disk, where anyone can type 4.0. Unclamped, one edit
    // would let a distant culture outweigh the subject.
    let guild = node(NodeKind::Culture, "Ember Guild");
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    weighted_link(&mut kael, &guild, LinkRole::MemberOf, 4.0);

    let world = World::new(vec![&guild, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    assert_eq!(stack.in_layer(Layer::Culture).next().unwrap().weight, 1.0);
}

#[test]
fn the_shot_layer_is_last_when_there_is_one_and_absent_when_there_is_not() {
    // Layer 7 comes from controls that do not exist yet. Resolving for display
    // has no shot at all, and that has to be a stack the Inspector can render
    // rather than an empty card.
    let ashfall = Ashfall::new();
    let world = ashfall.world();

    let with_shot = resolve(&world, ashfall.kael.id, Some(Shot::new("Portrait study"))).unwrap();
    let last = with_shot.sources().last().unwrap();
    assert_eq!((last.layer, last.name()), (Layer::Shot, "Portrait study"));
    assert_eq!(last.node_id(), None, "the shot is not a node");

    let without = resolve(&world, ashfall.kael.id, None).unwrap();
    assert_eq!(without.in_layer(Layer::Shot).count(), 0);
    assert_eq!(without.sources().last().unwrap().reached, Reached::Subject);
}

#[test]
fn a_world_of_one_thousand_nodes_resolves_in_well_under_a_millisecond() {
    // prompt_compile runs on every Inspector interaction, so this is a product
    // requirement rather than a nicety. The bound is loose enough not to flake on
    // a loaded CI box and tight enough to catch someone adding a scan of the
    // whole world to the walk.
    let style = node(NodeKind::StyleGuide, "Ashfall House Style");
    let bible = node(NodeKind::WorldBible, "Ashfall");
    let mut chain: Vec<Node> = Vec::new();
    for i in 0..1_000 {
        let mut setting = node(NodeKind::Setting, &format!("District {i}"));
        if let Some(previous) = chain.last() {
            setting.parent_id = Some(previous.id);
        }
        chain.push(setting);
    }
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    link(&mut kael, chain.last().unwrap(), LinkRole::LocatedIn);

    let mut nodes: Vec<&Node> = chain.iter().collect();
    nodes.extend([&style, &bible, &kael]);
    let world = World::new(nodes);

    let started = std::time::Instant::now();
    let stack = resolve(&world, kael.id, Some(Shot::new("Environment matte"))).unwrap();
    let elapsed = started.elapsed();

    assert_eq!(stack.sources().len(), 1_004);
    assert!(elapsed < std::time::Duration::from_millis(1), "took {elapsed:?}");
}
