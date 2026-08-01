//! Snapshot tests over what the layer cards would list.
//!
//! Same discipline as `stacks.rs`, and for the same reason: a weighting or
//! routing regression throws nothing and looks like nothing — the art just comes
//! out subtly wrong for months. So the tests write out the whole expected list —
//! layer, source, section, body, weight, target — longhand, and a diff on one is
//! meant to be read and argued with.

use wobu_core::{
    AssetRef, AssetRole, Description, FragmentTarget, Id, Layer, Link, LinkRole, Node, NodeKind,
    SectionValue, default_preset, kind_registry, preset,
};
use wobu_influence::{
    Fragment, FragmentBody, Shot, Sliders, World, fragments, fragments_for_view, resolve,
    section_target,
};

use FragmentBody::{Asset, Text};
use FragmentTarget::{MoodboardOnly, Negative, Prompt, StructureRef};

fn node(kind: NodeKind, name: &str) -> Node {
    Node::new(kind, name).expect("fixture names are sluggable")
}

fn describe(node: &mut Node, sections: impl IntoIterator<Item = (&'static str, SectionValue)>) {
    node.description = Some(Description::from_sections(
        sections.into_iter().map(|(key, value)| (key.to_string(), value)),
    ));
}

fn prose(value: &str) -> SectionValue {
    SectionValue::Text(value.to_string())
}

fn list(items: &[&str]) -> SectionValue {
    SectionValue::List(items.iter().map(|i| i.to_string()).collect())
}

/// One row of a layer card: where it came from, what it is, and where it goes.
type Row<'a> = (Layer, &'a str, &'static str, FragmentBody<'a>, f32, FragmentTarget);

fn snapshot<'a>(fragments: &[Fragment<'a>]) -> Vec<Row<'a>> {
    fragments
        .iter()
        .map(|f| (f.layer(), f.source_name(), f.section(), f.body(), f.weight(), f.target()))
        .collect()
}

/// Three layers with a description on every card, plus a reference the user is
/// allowed to see and one they are not. The smallest world that exercises the
/// layer order, the kind's section order, both text targets and both image
/// outcomes at once.
struct Ashfall {
    style: Node,
    vashk: Node,
    kael: Node,
    /// Attached to the subject as `pose`, so it is conditioning.
    pose_ref: Id,
    /// Attached to the subject as `mood`, so it must never be sent anywhere.
    mood_ref: Id,
}

impl Ashfall {
    fn new() -> Ashfall {
        let mut style = node(NodeKind::StyleGuide, "Ashfall House Style");
        describe(
            &mut style,
            [("medium", prose("Oil on board")), ("never", list(&["photographic detail"]))],
        );

        let mut vashk = node(NodeKind::Species, "Vashk");
        describe(
            &mut vashk,
            [("silhouette", prose("Long-limbed, four-jointed")), ("never", list(&["fur"]))],
        );

        let pose_ref = wobu_core::new_id();
        let mood_ref = wobu_core::new_id();
        let mut kael = node(NodeKind::Character, "Kael Vantris");
        describe(
            &mut kael,
            [
                ("silhouette", prose("Tall, narrow, hooded")),
                ("costume", prose("Ash-grey longcoat")),
                ("palette", list(&["#2b2118", "#c2703a"])),
                ("never", list(&["modern firearms", "clean surfaces"])),
            ],
        );
        kael.links.push(Link::new(vashk.id, LinkRole::SpeciesOf));
        kael.asset_links.push(AssetRef::new(pose_ref, AssetRole::Pose));
        kael.asset_links.push(AssetRef::new(mood_ref, AssetRole::Mood));

        Ashfall { style, vashk, kael, pose_ref, mood_ref }
    }

    fn nodes(&self) -> Vec<&Node> {
        vec![&self.style, &self.vashk, &self.kael]
    }

    fn world(&self) -> World<'_> {
        World::new(self.nodes())
    }
}

#[test]
fn a_character_sheet_reads_out_layer_by_layer_and_section_by_section() {
    // The snapshot the rest of the file is written against: every fragment of a
    // real stack, in the order the Inspector lists them and the compiler joins
    // them — outermost layer first, the subject last before the shot, and within
    // a source the order its kind declares its sections.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Character sheet · 3:4"))).unwrap();
    let sheet = default_preset(NodeKind::Character);
    let compiled = fragments(&stack, sheet, &Sliders::neutral());

    assert_eq!(
        snapshot(&compiled),
        vec![
            (Layer::Style, "Ashfall House Style", "medium", Text("Oil on board"), 1.0, Prompt),
            (
                Layer::Style,
                "Ashfall House Style",
                "never",
                Text("photographic detail"),
                1.0,
                Negative
            ),
            // A character sheet is read as a shape, so `silhouette` is the section it
            // leans on hardest — 1.4 wherever it appears, at any depth in the stack.
            (
                Layer::Ancestry,
                "Vashk",
                "silhouette",
                Text("Long-limbed, four-jointed"),
                1.4,
                Prompt
            ),
            (Layer::Ancestry, "Vashk", "never", Text("fur"), 1.0, Negative),
            (
                Layer::Subject,
                "Kael Vantris",
                "silhouette",
                Text("Tall, narrow, hooded"),
                1.4,
                Prompt
            ),
            (Layer::Subject, "Kael Vantris", "costume", Text("Ash-grey longcoat"), 1.3, Prompt),
            // A list section is one fragment per item, not one per list.
            (Layer::Subject, "Kael Vantris", "palette", Text("#2b2118"), 1.0, Prompt),
            (Layer::Subject, "Kael Vantris", "palette", Text("#c2703a"), 1.0, Prompt),
            (Layer::Subject, "Kael Vantris", "never", Text("modern firearms"), 1.0, Negative),
            (Layer::Subject, "Kael Vantris", "never", Text("clean surfaces"), 1.0, Negative),
            // References follow the prose of the node they hang off, routed by role.
            // The role travels on the body as well as being the section key, because
            // the image budget maps roles to a backend's reference buckets and must
            // not read one back out of a string (#44).
            (
                Layer::Subject,
                "Kael Vantris",
                "pose",
                Asset { id: ashfall.pose_ref, role: AssetRole::Pose },
                1.0,
                StructureRef,
            ),
            (
                Layer::Subject,
                "Kael Vantris",
                "mood",
                Asset { id: ashfall.mood_ref, role: AssetRole::Mood },
                1.0,
                MoodboardOnly,
            ),
            // The framing text is the preset's own product copy and it lives in
            // `wobu-core`; a second copy here would fail this snapshot for a wording
            // edit that has nothing to do with extraction.
            (Layer::Shot, "Character sheet · 3:4", "framing", Text(sheet.framing), 1.0, Prompt),
        ]
    );
}

#[test]
fn a_mood_reference_is_on_the_moodboard_and_reaches_no_backend() {
    // The one that would be a privacy incident rather than a bug. `wobu-core`
    // proved it at the link layer; this proves the fragment layer did not lose
    // it. A mood board is what an artist collects for themselves, and every
    // other role is on a path that ends at somebody else's servers.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Character sheet"))).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    // It is a fragment: the human sees it on the moodboard, and the layer card
    // counts it among what this node contributes.
    let mood = compiled.iter().find(|f| f.asset_id() == Some(ashfall.mood_ref)).unwrap();
    assert_eq!(mood.target(), MoodboardOnly);
    assert_eq!(mood.node_id(), Some(ashfall.kael.id), "and it is still attributed");
    assert!(!mood.is_sendable());

    // And nothing in the set a provider would be handed carries it.
    for fragment in compiled.iter().filter(|f| f.is_sendable()) {
        assert_ne!(fragment.asset_id(), Some(ashfall.mood_ref), "{fragment:?}");
    }
    // Stated the other way too, so that a filter which dropped *every* reference
    // would fail rather than pass this test by accident.
    assert!(
        compiled.iter().filter(|f| f.is_sendable()).any(|f| f.asset_id() == Some(ashfall.pose_ref)),
        "the pose reference is conditioning and must survive"
    );
}

#[test]
fn every_role_routes_its_reference_the_way_the_link_layer_says() {
    // The mapping belongs to `AssetRole::target` and this crate must not have
    // grown a second copy of it. Roles are attached to an upstream layer as well
    // as the subject, because "mood is dropped" implemented as "the subject's
    // mood is dropped" would leak every inherited one.
    let mut vashk = node(NodeKind::Species, "Vashk");
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link::new(vashk.id, LinkRole::SpeciesOf));

    let mut references: Vec<(Layer, AssetRole, Id)> = Vec::new();
    for role in AssetRole::ALL {
        let id = wobu_core::new_id();
        vashk.asset_links.push(AssetRef::new(id, role));
        references.push((Layer::Ancestry, role, id));
    }
    for role in AssetRole::ALL {
        let id = wobu_core::new_id();
        kael.asset_links.push(AssetRef::new(id, role));
        references.push((Layer::Subject, role, id));
    }

    let world = World::new(vec![&vashk, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    let routed: Vec<_> = compiled.iter().map(|f| (f.layer(), f.section(), f.target())).collect();
    let expected: Vec<_> =
        references.iter().map(|(layer, role, _)| (*layer, role.as_str(), role.target())).collect();
    assert_eq!(routed, expected);

    // Every mood reference in the stack, inherited or not, is held back; every
    // other one goes through.
    for (_, role, id) in references {
        let fragment = compiled.iter().find(|f| f.asset_id() == Some(id)).unwrap();
        assert_eq!(fragment.is_sendable(), role != AssetRole::Mood, "{role}");
    }
}

#[test]
fn a_disabled_reference_contributes_nothing_at_all() {
    // `enabled` is the world's own off switch, the same one `resolve` honours on
    // influence links. A disabled reference that still produced a fragment would
    // make the toggle in the References tab look broken — and, for a role that
    // is conditioning, would send a picture the user had turned off.
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    let off = wobu_core::new_id();
    kael.asset_links.push(AssetRef { enabled: false, ..AssetRef::new(off, AssetRole::FullRef) });

    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    assert!(compiled.is_empty());
}

#[test]
fn the_never_section_is_the_only_negative_in_the_whole_vocabulary() {
    // Pinned longhand over every section any kind declares, so that adding one
    // to the kind registry fails here until somebody decides which way it goes.
    // A negative that compiled into the positive prompt would ask for exactly
    // what it was written to forbid, and nothing would report it.
    let mut vocabulary: Vec<(&str, FragmentTarget)> = kind_registry()
        .iter()
        .flat_map(|def| def.sections)
        .map(|s| (s.key, section_target(s.key)))
        .collect();
    vocabulary.sort_unstable_by_key(|(key, _)| *key);
    vocabulary.dedup();

    assert_eq!(
        vocabulary,
        vec![
            ("anatomy", Prompt),
            ("architecture", Prompt),
            ("climate", Prompt),
            ("costume", Prompt),
            ("era", Prompt),
            ("iconography", Prompt),
            ("light", Prompt),
            ("lighting", Prompt),
            ("line_quality", Prompt),
            ("materials", Prompt),
            ("medium", Prompt),
            ("never", Negative),
            ("ornament", Prompt),
            // Hex strings are words in the prompt. `FragmentTarget::Palette` is the
            // colour-conditioning channel and what arrives there is images.
            ("palette", Prompt),
            ("rendering", Prompt),
            ("signature", Prompt),
            ("silhouette", Prompt),
            ("tech_level", Prompt),
            ("tone", Prompt),
            ("weapons", Prompt),
            ("wear", Prompt),
        ]
    );
}

#[test]
fn the_preset_is_what_decides_which_sections_matter() {
    // The worked example from `docs/04-influence-engine.md`: a material study
    // boosts `materials` and drops `silhouette`, a turnaround does the reverse.
    // Same world, same stack, same sliders — only the sheet being asked for
    // changes, and that has to be enough to change the weights.
    let mut lantern = node(NodeKind::Prop, "Ashglass Lantern");
    describe(
        &mut lantern,
        [
            ("silhouette", prose("A squat hexagonal cage")),
            ("materials", prose("Blown ashglass over blackened iron")),
        ],
    );

    let world = World::new([&lantern]);
    let stack = resolve(&world, lantern.id, None).unwrap();

    let weights = |id: &str| -> Vec<(&'static str, f32)> {
        fragments(&stack, preset(id).unwrap(), &Sliders::neutral())
            .iter()
            .map(|f| (f.section(), f.weight()))
            .collect()
    };

    assert_eq!(weights("material_study"), vec![("silhouette", 0.3), ("materials", 2.0)]);
    assert_eq!(weights("turnaround"), vec![("silhouette", 1.6), ("materials", 0.6)]);
}

#[test]
fn a_preset_and_a_kind_that_disagree_about_a_section_are_both_ordinary() {
    // Two half-empty cases that must not be errors, because both are the normal
    // state of a registry with eight presets and ten kinds. A costume plate
    // weights `costume` and `iconography`; a prop declares neither, and declares
    // `wear`, which the costume plate says nothing about.
    let mut lantern = node(NodeKind::Prop, "Ashglass Lantern");
    describe(&mut lantern, [("wear", prose("Soot in every seam"))]);

    let world = World::new([&lantern]);
    let stack = resolve(&world, lantern.id, None).unwrap();
    let compiled = fragments(&stack, preset("costume_plate").unwrap(), &Sliders::neutral());

    // The section the preset never mentions is left at the registry's documented
    // 1.0, and the sections the preset boosts simply never come up.
    assert_eq!(
        snapshot(&compiled),
        vec![(Layer::Subject, "Ashglass Lantern", "wear", Text("Soot in every seam"), 1.0, Prompt)]
    );
}

#[test]
fn a_weight_is_the_path_the_section_priority_and_the_slider_multiplied_out() {
    // The formula from `docs/04-influence-engine.md`, with all three terms set to
    // something other than 1.0 at once so that a dropped term is visible. Any one
    // of them silently missing is a stack that still compiles and still renders,
    // just not the way the user weighted it.
    let mut vashk = node(NodeKind::Species, "Vashk");
    describe(
        &mut vashk,
        [("silhouette", prose("Long-limbed, four-jointed")), ("never", list(&["fur"]))],
    );

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    // Held to its species at half strength: the path product.
    kael.links.push(Link { weight: 0.5, ..Link::new(vashk.id, LinkRole::SpeciesOf) });

    let world = World::new(vec![&vashk, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    // And the user pulled that card's slider to half as well.
    let sliders = Sliders::from_pairs([(vashk.id, 0.5)]);
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &sliders);

    let weights: Vec<_> = compiled.iter().map(|f| (f.section(), f.weight())).collect();
    assert_eq!(
        weights,
        vec![
            // 0.5 path × 1.4 character-sheet silhouette × 0.5 slider.
            ("silhouette", 0.35),
            // 0.5 path × 1.0 (no opinion) × 0.5 slider.
            ("never", 0.25),
        ]
    );
}

#[test]
fn a_references_own_weight_is_the_last_edge_of_the_path() {
    // An image attached at half strength to a culture the subject is held to at
    // half strength is a quarter of the way in — the same multiply-along-the-chain
    // rule influence links follow. Taking only the last edge would let a
    // deliberately loose link be undone by a firmly attached picture.
    let picture = wobu_core::new_id();
    let mut guild = node(NodeKind::Culture, "Ember Guild");
    guild.asset_links.push(AssetRef { weight: 0.5, ..AssetRef::new(picture, AssetRole::Costume) });

    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link { weight: 0.5, ..Link::new(guild.id, LinkRole::MemberOf) });

    let world = World::new(vec![&guild, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    // A costume plate leans on costume references as hard as on costume prose,
    // because the role names the same section the preset weights.
    let compiled = fragments(&stack, preset("costume_plate").unwrap(), &Sliders::neutral());

    let weights: Vec<_> = compiled.iter().map(|f| (f.section(), f.weight())).collect();
    assert_eq!(weights, vec![("costume", 0.25 * 1.8)]);
}

#[test]
fn a_hand_edited_reference_weight_outside_the_range_cannot_amplify_a_layer() {
    // Asset weights come off disk, where anyone can type 4.0. Unclamped, one edit
    // would let a distant layer's reference outweigh the subject's own.
    let picture = wobu_core::new_id();
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.asset_links.push(AssetRef { weight: 4.0, ..AssetRef::new(picture, AssetRole::FullRef) });

    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    assert_eq!(compiled[0].weight(), 1.0);
}

#[test]
fn a_layer_turned_all_the_way_down_keeps_its_fragments_for_attribution() {
    // A slider at zero means "contributes nothing", not "was never written". The
    // Inspector's job is explaining where the prompt came from, and a card whose
    // rows disappeared when it was turned down would be explaining the opposite —
    // so extraction reports them and the compiler skips them on `contributes`.
    let ashfall = Ashfall::new();
    let world = ashfall.world();
    let stack = resolve(&world, ashfall.kael.id, None).unwrap();
    let sliders = Sliders::from_pairs([(ashfall.vashk.id, 0.0)]);
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &sliders);

    let ancestry: Vec<_> = compiled
        .iter()
        .filter(|f| f.layer() == Layer::Ancestry)
        .map(|f| (f.section(), f.weight(), f.contributes()))
        .collect();
    assert_eq!(ancestry, vec![("silhouette", 0.0, false), ("never", 0.0, false)]);

    // Turning one card down changes nothing about any other card.
    assert!(
        compiled.iter().filter(|f| f.layer() == Layer::Subject).all(|f| f.contributes()),
        "the subject was not touched"
    );
}

#[test]
fn the_shot_layer_contributes_the_presets_framing_text() {
    // Framing is prose compiled alongside everything else rather than a parameter
    // a backend understands, so it has to arrive as a fragment — otherwise the
    // pose and lighting instructions are the one part of the prompt the Inspector
    // cannot attribute to anything.
    let kael = node(NodeKind::Character, "Kael Vantris");
    let world = World::new([&kael]);
    let portrait = preset("portrait_study").unwrap();

    let with_shot = resolve(&world, kael.id, Some(Shot::new("Portrait study · 4:5"))).unwrap();
    let compiled = fragments(&with_shot, portrait, &Sliders::neutral());
    assert_eq!(
        snapshot(&compiled),
        vec![(Layer::Shot, "Portrait study · 4:5", "framing", Text(portrait.framing), 1.0, Prompt)]
    );
    assert_eq!(compiled[0].node_id(), None, "the shot is not a node");

    // Resolved for display rather than for a generation there is no shot at all,
    // and so nothing has been framed yet.
    let without = resolve(&world, kael.id, None).unwrap();
    assert!(fragments(&without, portrait, &Sliders::neutral()).is_empty());
}

#[test]
fn each_turnaround_generation_appends_its_tagged_view_framing() {
    let lantern = node(NodeKind::Prop, "Ashglass Lantern");
    let world = World::new([&lantern]);
    let turnaround = preset("turnaround").unwrap();
    let stack = resolve(&world, lantern.id, Some(Shot::new("Turnaround"))).unwrap();

    for planned in turnaround.generations(42) {
        let view = planned.view.expect("every turnaround generation is tagged");
        let extracted = fragments_for_view(&stack, turnaround, &Sliders::neutral(), view);
        assert_eq!(
            snapshot(&extracted),
            vec![
                (Layer::Shot, "Turnaround", "framing", Text(turnaround.framing), 1.0, Prompt),
                (Layer::Shot, "Turnaround", "view_framing", Text(view.framing), 1.0, Prompt),
            ],
            "{}",
            view.view_type,
        );
        assert_eq!(planned.seed, 42);
    }
}

#[test]
fn nothing_blank_becomes_a_fragment() {
    // Half-filled descriptions are the normal state between an enhance finishing
    // and the user editing it. A blank fragment is an empty row on the layer
    // card, an empty span in the prompt, and budget spent on nothing.
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    describe(
        &mut kael,
        [
            ("silhouette", prose("   ")),
            ("costume", prose("  Ash-grey longcoat  ")),
            ("never", list(&["", "  ", "modern firearms"])),
        ],
    );

    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    // Surviving text is trimmed, because the compiler joins fragments with its
    // own separator and leading space would double it.
    assert_eq!(
        snapshot(&compiled),
        vec![
            (Layer::Subject, "Kael Vantris", "costume", Text("Ash-grey longcoat"), 1.3, Prompt),
            (Layer::Subject, "Kael Vantris", "never", Text("modern firearms"), 1.0, Negative),
        ]
    );
}

#[test]
fn a_node_with_no_description_yet_still_contributes_its_references() {
    // Every node is in this state from the moment it is created until the first
    // enhance, and dragging reference images in is the first thing people do.
    let picture = wobu_core::new_id();
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.asset_links.push(AssetRef::new(picture, AssetRole::FullRef));
    assert!(kael.description.is_none());

    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    assert_eq!(compiled.len(), 1);
    assert_eq!(compiled[0].asset_id(), Some(picture));
}

#[test]
fn a_section_the_kind_does_not_declare_is_not_compiled() {
    // The store already drops these when it reads a file
    // (`Description::normalised_for`), so one can only reach here from a caller
    // that built a description by hand. Compiling it anyway would mean a section
    // the editor refuses to show could still change the picture, which is
    // unarguable from the UI.
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    describe(
        &mut kael,
        [("climate", prose("Ash-choked and humid")), ("silhouette", prose("Tall, narrow, hooded"))],
    );

    let world = World::new([&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let compiled = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    let sections: Vec<_> = compiled.iter().map(|f| f.section()).collect();
    assert_eq!(sections, vec!["silhouette"]);
}

#[test]
fn sections_compile_in_the_kinds_declared_order_however_the_file_was_written() {
    // A hand-edited file with its sections shuffled must compile to the same
    // prompt as one the app wrote, or two people with the same world get
    // different art and nothing on screen explains why.
    let mut written = node(NodeKind::Character, "Kael Vantris");
    describe(
        &mut written,
        [
            ("silhouette", prose("Tall, narrow, hooded")),
            ("costume", prose("Ash-grey longcoat")),
            ("never", list(&["modern firearms"])),
        ],
    );

    let mut shuffled = node(NodeKind::Character, "Kael Vantris");
    describe(
        &mut shuffled,
        [
            ("never", list(&["modern firearms"])),
            ("costume", prose("Ash-grey longcoat")),
            ("silhouette", prose("Tall, narrow, hooded")),
        ],
    );

    let sheet = default_preset(NodeKind::Character);
    let compile = |node: &Node| {
        let world = World::new([node]);
        let stack = resolve(&world, node.id, None).unwrap();
        format!("{:?}", snapshot(&fragments(&stack, sheet, &Sliders::neutral())))
    };

    assert_eq!(compile(&written), compile(&shuffled));
    assert!(compile(&written).contains("silhouette"));
}

#[test]
fn fragments_do_not_depend_on_the_order_the_nodes_were_loaded() {
    // The "works on my machine" bug this crate is most exposed to, stated again
    // one stage further down: the caller hands its nodes over in whatever order
    // its own map iterates in, and the prompt comes out different on the next
    // launch with nothing in the world to explain it.
    let ashfall = Ashfall::new();
    let sheet = default_preset(NodeKind::Character);
    let sliders = Sliders::from_pairs([(ashfall.vashk.id, 0.25)]);
    let compile = |nodes: Vec<&Node>| {
        let world = World::new(nodes);
        let stack = resolve(&world, ashfall.kael.id, Some(Shot::new("Character sheet"))).unwrap();
        format!("{:?}", snapshot(&fragments(&stack, sheet, &sliders)))
    };

    let baseline = compile(ashfall.nodes());
    let mut nodes = ashfall.nodes();
    for rotation in 0..nodes.len() {
        nodes.rotate_left(1);
        assert_eq!(compile(nodes.clone()), baseline, "rotation {rotation}");
    }
    nodes.reverse();
    assert_eq!(compile(nodes), baseline, "reversed");

    // And the same inputs twice in a row are byte-identical, which is what makes
    // an `influence_snapshot` reproducible six months later.
    assert_eq!(compile(ashfall.nodes()), baseline);
}

#[test]
fn an_absurd_stack_in_a_large_world_compiles_within_the_interactive_budget() {
    // `prompt_compile` runs on every Inspector interaction — every drag of a
    // weight slider — so this is a product requirement rather than a nicety
    // (`docs/05-architecture.md`), and it is the same bound `stacks.rs` holds
    // resolution to.
    //
    // The two sizes are different on purpose, because the cost has two possible
    // shapes and only one of them is a bug. Extraction is proportional to the
    // *stack*, and a stack is bounded by the user's own hierarchy: the deepest
    // chain the data model describes is Region → City → District, so a hundred
    // layer cards is already an order of magnitude past anything a real world
    // produces. The *world* around it is a thousand nodes, of which nine hundred
    // are unreachable from the subject — so if anyone ever replaces the walk over
    // `stack.sources()` with a walk over the world, the fragment count assertion
    // catches it and this timing does too.
    let mut chain: Vec<Node> = Vec::new();
    let mut bystanders: Vec<Node> = Vec::new();
    for i in 0..1_000 {
        let mut setting = node(NodeKind::Setting, &format!("District {i}"));
        describe(
            &mut setting,
            [
                ("climate", prose("Ash-choked and humid")),
                ("architecture", prose("Stilted timber over black water")),
                ("light", prose("Low sun through smoke")),
                ("wear", prose("Salt-rotted and soot-stained")),
                ("materials", prose("Tar, rope, blown glass")),
                ("palette", list(&["#2b2118", "#c2703a", "#4fd1c5"])),
                ("never", list(&["clean surfaces", "modern firearms"])),
            ],
        );
        setting.asset_links.push(AssetRef::new(wobu_core::new_id(), AssetRole::Mood));
        if i < 100 {
            if let Some(previous) = chain.last() {
                setting.parent_id = Some(previous.id);
            }
            chain.push(setting);
        } else {
            bystanders.push(setting);
        }
    }
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link::new(chain.last().unwrap().id, LinkRole::LocatedIn));

    let mut nodes: Vec<&Node> = chain.iter().chain(bystanders.iter()).collect();
    nodes.push(&kael);
    let world = World::new(nodes);
    let stack = resolve(&world, kael.id, Some(Shot::new("Environment matte"))).unwrap();
    let sheet = preset("environment_matte").unwrap();
    assert_eq!(stack.sources().len(), 102, "a hundred places, the subject and the shot");

    // Fastest of several runs, not one — a single sample measures the machine's
    // load as much as this code, and the neighbouring resolve bound in
    // `stacks.rs` has failed that way with a build running beside it. The
    // regression worth catching here is a walk over the world rather than the
    // stack, which is an order of magnitude and fails the fastest run too.
    let mut fastest = std::time::Duration::MAX;
    let mut compiled = Vec::new();
    for _ in 0..5 {
        let started = std::time::Instant::now();
        compiled = fragments(&stack, sheet, &Sliders::neutral());
        fastest = fastest.min(started.elapsed());
    }

    // Ten fragments and one reference per district, plus the framing text. The
    // subject has no description of its own yet.
    assert_eq!(compiled.len(), 1_101);
    // Shared CI runners regularly add about a millisecond of scheduler noise.
    // Five milliseconds remains an interactive budget and still catches the
    // order-of-magnitude full-world walk this benchmark guards against.
    assert!(fastest < std::time::Duration::from_millis(5), "took {fastest:?}");
}
