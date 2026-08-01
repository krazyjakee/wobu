//! Which reference images survive the backend's caps, and which card is told it
//! lost one.
//!
//! Same discipline as `prompts.rs`, one channel over: a budget that drops the
//! wrong reference throws nothing and looks like nothing — a picture the user
//! attached simply is not in the request, and the art comes back subtly wrong.
//! So the kept lists and the whole drop report are written out longhand, and a
//! diff on one is meant to be read and argued with.

use wobu_core::{AssetRef, AssetRole, Id, Layer, Link, LinkRole, Node, NodeKind, default_preset};
use wobu_influence::{
    Bucket, Budget, Chars, CompiledImages, DropReason, Fragment, ImageBudget, RefBucket, Refs,
    Shot, Sliders, World, compile, compile_images, fragments, image_budget, resolve,
};

use DropReason::{Budget as Cut, Silenced};

fn node(kind: NodeKind, name: &str) -> Node {
    Node::new(kind, name).expect("fixture names are sluggable")
}

/// One row of a bucket: which card the reference hangs off, and which reference.
type Row<'a> = (Layer, &'a str, &'static str);

/// One line of a bucket's drop report: the same, and why it is not being sent.
type Casualty<'a> = (Layer, &'a str, &'static str, DropReason);

/// A stack that offers more references than `gemini-3-pro-image` takes, spread
/// over three layers so that "which card lost one" is a question with an answer,
/// and covering every role including the one that may never be sent.
struct Ashfall {
    style: Node,
    vashk: Node,
    kael: Node,
    /// Every reference in the fixture, with a name to read it back by. Two
    /// references in the same role on the same node are otherwise
    /// indistinguishable in a report, and telling them apart is the whole point
    /// of a test about which one was dropped.
    named: Vec<(&'static str, Id)>,
}

impl Ashfall {
    fn new() -> Ashfall {
        let mut style = node(NodeKind::StyleGuide, "Ashfall House Style");
        let mut vashk = node(NodeKind::Species, "Vashk");
        let mut kael = node(NodeKind::Character, "Kael Vantris");
        kael.links.push(Link::new(vashk.id, LinkRole::SpeciesOf));

        let mut named: Vec<(&'static str, Id)> = Vec::new();
        let mut attach = |owner: &mut Node, name: &'static str, role: AssetRole| {
            let id = wobu_core::new_id();
            owner.asset_links.push(AssetRef::new(id, role));
            named.push((name, id));
        };

        attach(&mut style, "house material", AssetRole::Material);
        attach(&mut style, "house grain", AssetRole::Material);
        attach(&mut vashk, "vashk silhouette", AssetRole::Silhouette);
        attach(&mut vashk, "vashk pose", AssetRole::Pose);
        attach(&mut kael, "kael costume", AssetRole::Costume);
        attach(&mut kael, "kael cloak", AssetRole::Costume);
        attach(&mut kael, "kael portrait", AssetRole::FullRef);
        attach(&mut kael, "kael palette", AssetRole::Palette);
        attach(&mut kael, "kael pose", AssetRole::Pose);
        attach(&mut kael, "kael moodboard", AssetRole::Mood);

        Ashfall { style, vashk, kael, named }
    }

    fn nodes(&self) -> Vec<&Node> {
        vec![&self.style, &self.vashk, &self.kael]
    }

    /// Everything the stack contributes, unsorted and unfiltered, which is what
    /// `compile_images` is specified to be handed — the same slice `compile` is.
    fn extract<'a>(&'a self, world: &World<'a>, sliders: &Sliders) -> Vec<Fragment<'a>> {
        let shot = Shot::new("Character sheet · 3:4");
        let stack = resolve(world, self.kael.id, Some(shot)).unwrap();
        fragments(&stack, default_preset(NodeKind::Character), sliders)
    }

    fn name(&self, fragment: &Fragment<'_>) -> &'static str {
        let id = fragment.asset_id().expect("only references are in these buckets");
        self.named.iter().find(|(_, known)| *known == id).expect("built by this fixture").0
    }

    fn kept<'a>(&'a self, bucket: &Bucket<'a>) -> Vec<Row<'a>> {
        bucket.kept().iter().map(|f| (f.layer(), f.source_name(), self.name(f))).collect()
    }

    fn casualties<'a>(&'a self, bucket: &Bucket<'a>) -> Vec<Casualty<'a>> {
        let named = |f: &Fragment<'a>| (f.layer(), f.source_name(), self.name(f));
        bucket
            .dropped()
            .iter()
            .map(|d| {
                let (layer, card, name) = named(&d.fragment);
                (layer, card, name, d.reason)
            })
            .collect()
    }
}

/// The counters, in the order the report lists them: which bucket, how many are
/// being sent, and out of how many.
fn counters(images: &CompiledImages<'_>) -> Vec<(RefBucket, usize, Option<usize>)> {
    images.buckets().iter().map(|b| (b.bucket(), b.kept().len(), b.cap().limit())).collect()
}

#[test]
fn references_are_sorted_into_the_buckets_the_model_counts_them_in() {
    // The snapshot the rest of the file is written against, against the only
    // model that declares all three buckets. Two things are being pinned at once:
    // which bucket each role competes in — a `pose` reference filed as a style ref
    // would evict the `full_ref` that pins the subject's appearance — and that the
    // three pools are counted separately, so a stack rich in objects cannot cost
    // the subject a style reference.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let images = compile_images(&extracted, image_budget("gemini-3-pro-image").unwrap());

    assert_eq!(
        counters(&images),
        vec![
            (RefBucket::Objects, 2, Some(6)),
            (RefBucket::Characters, 2, Some(5)),
            (RefBucket::StyleRefs, 3, Some(3)),
        ]
    );

    // Objects: a silhouette is a shape, which is as likely to be a building as a
    // person, and a palette swatch is a picture of a thing. Neither is close to
    // the cap, so nothing here is dropped.
    let objects = images.bucket(RefBucket::Objects).unwrap();
    assert_eq!(
        ashfall.kept(objects),
        vec![
            (Layer::Ancestry, "Vashk", "vashk silhouette"),
            (Layer::Subject, "Kael Vantris", "kael palette"),
        ]
    );
    assert_eq!(ashfall.casualties(objects), vec![]);

    // Characters: the pose references, inherited one included.
    let characters = images.bucket(RefBucket::Characters).unwrap();
    assert_eq!(
        ashfall.kept(characters),
        vec![
            (Layer::Ancestry, "Vashk", "vashk pose"),
            (Layer::Subject, "Kael Vantris", "kael pose"),
        ]
    );
    assert_eq!(ashfall.casualties(characters), vec![]);

    // Style refs: five offered, three taken. A character sheet leans on the
    // subject's own costume, so the house style's two surface references are the
    // lightest and they are what goes — and the survivors come back in reading
    // order, not in the weight order the drop decision was made in.
    let style_refs = images.bucket(RefBucket::StyleRefs).unwrap();
    assert_eq!(
        ashfall.kept(style_refs),
        vec![
            (Layer::Subject, "Kael Vantris", "kael costume"),
            (Layer::Subject, "Kael Vantris", "kael cloak"),
            (Layer::Subject, "Kael Vantris", "kael portrait"),
        ]
    );
    assert_eq!(
        ashfall.casualties(style_refs),
        vec![
            (Layer::Style, "Ashfall House Style", "house material", Cut),
            (Layer::Style, "Ashfall House Style", "house grain", Cut),
        ]
    );

    // The mood board is in none of them, kept or dropped. It is not a casualty —
    // it is doing exactly what it was attached to do.
    assert!(!images.kept().any(|f| ashfall.name(&f) == "kael moodboard"));
    assert!(!images.dropped().any(|d| ashfall.name(&d.fragment) == "kael moodboard"));
}

#[test]
fn the_card_that_lost_a_reference_is_the_one_that_can_say_so() {
    // The acceptance criterion of #44, built out of the report exactly as the
    // Inspector will build it (#47): the bucket knows its cap and what it is
    // sending, and every casualty carries the whole fragment, so it still knows
    // which layer card it came off. A report that listed only totals would put
    // this sentence on no card, and a flat list of casualties would put it on all
    // of them.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let images = compile_images(&extracted, image_budget("gemini-3-pro-image").unwrap());

    let bucket = images.bucket(RefBucket::StyleRefs).unwrap();
    let sentence = |layer: Layer| {
        let lost = bucket.dropped().iter().filter(|d| d.fragment.layer() == layer).count();
        format!(
            "{}/{} {} · {lost} dropped",
            bucket.kept().len(),
            bucket.cap().get(),
            bucket.bucket().label(),
        )
    };
    assert_eq!(sentence(Layer::Style), "3/3 style refs · 2 dropped");
    assert_eq!(sentence(Layer::Subject), "3/3 style refs · 0 dropped");

    // And the node behind the card, not just its layer — an ancestry chain is
    // several cards on one layer, so the panel groups by source.
    let house = Some(ashfall.style.id);
    assert_eq!(bucket.dropped().iter().filter(|d| d.fragment.node_id() == house).count(), 2);
}

#[test]
fn a_model_that_does_not_separate_a_bucket_still_takes_the_references() {
    // The other half of #44's judgement, and the one that would fail silently.
    // A `–` in the providers table is not a zero: `gemini-3.1-flash-lite-image`
    // does not refuse style or character references, it just has one
    // undifferentiated category of fourteen. Read as "unsupported", this fixture
    // would lose seven references the user deliberately attached, and the only
    // trace would be a drop report nobody reads until the art is wrong.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());

    let lite = compile_images(&extracted, image_budget("gemini-3.1-flash-lite-image").unwrap());
    assert_eq!(counters(&lite), vec![(RefBucket::Objects, 9, Some(14))], "one category, nine refs");
    assert_eq!(lite.dropped().count(), 0);
    assert_eq!(
        ashfall.kept(lite.bucket(RefBucket::Objects).unwrap()),
        vec![
            (Layer::Style, "Ashfall House Style", "house material"),
            (Layer::Style, "Ashfall House Style", "house grain"),
            (Layer::Ancestry, "Vashk", "vashk silhouette"),
            (Layer::Ancestry, "Vashk", "vashk pose"),
            (Layer::Subject, "Kael Vantris", "kael costume"),
            (Layer::Subject, "Kael Vantris", "kael cloak"),
            (Layer::Subject, "Kael Vantris", "kael portrait"),
            (Layer::Subject, "Kael Vantris", "kael palette"),
            (Layer::Subject, "Kael Vantris", "kael pose"),
        ]
    );

    // The middle model separates characters out but not style, so the style
    // references join the object pool and the two pose references get their own.
    // The report's shape is the backend's capability: as many counters as it has
    // categories, which is what the Inspector shows.
    let flash = compile_images(&extracted, image_budget("gemini-3.1-flash-image").unwrap());
    assert_eq!(
        counters(&flash),
        vec![(RefBucket::Objects, 7, Some(10)), (RefBucket::Characters, 2, Some(4)),]
    );
    assert_eq!(flash.dropped().count(), 0);
    assert!(flash.bucket(RefBucket::StyleRefs).is_none(), "it does not meter that separately");

    // Metered as objects means sharing the object slots, not being handed a pool
    // of the same size. A budget that answered ten objects *and* ten style refs
    // would build a request of twenty pictures for a model that takes ten, and
    // the provider would be the one to say so, after the request was paid for.
    let crowded = ImageBudget { objects: Refs::new(4), characters: None, style_refs: None };
    let crowded = compile_images(&extracted, crowded);
    assert_eq!(counters(&crowded), vec![(RefBucket::Objects, 4, Some(4))]);
    assert_eq!(crowded.dropped().count(), 5);
}

#[test]
fn a_moodboard_reference_occupies_no_slot_and_is_never_reported_as_a_casualty() {
    // The property #26, #42 and #43 each preserved, at the stage where losing it
    // would cost a real reference its place. Stated as an equivalence rather than
    // as an arithmetic assertion, so it survives an edit to the fixture: budgeting
    // with the mood board and budgeting with it already removed must produce the
    // same answer, down to the report — and it is held at a cap tight enough that
    // one extra picture in the pool would change which pictures are sent.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let sendable: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.is_sendable()).copied().collect();
    assert_eq!(extracted.len(), sendable.len() + 1, "the mood board, and nothing else");

    for model in ["gemini-3.1-flash-lite-image", "gemini-3.1-flash-image", "gemini-3-pro-image"] {
        let budget = image_budget(model).unwrap();
        let (with, without) =
            (compile_images(&extracted, budget), compile_images(&sendable, budget));
        assert_eq!(with, without, "{model}");
    }
    let tightest = ImageBudget { objects: Refs::new(1), characters: None, style_refs: None };
    assert_eq!(compile_images(&extracted, tightest), compile_images(&sendable, tightest));

    // Which is worth saying in the direction the Inspector reads, too: a mood
    // reference is in no bucket at all, so no panel can report it as lost.
    let images = compile_images(&extracted, tightest);
    for bucket in images.buckets() {
        let sent = ashfall.kept(bucket);
        let lost = ashfall.casualties(bucket);
        assert!(!sent.iter().any(|(_, _, name)| *name == "kael moodboard"));
        assert!(!lost.iter().any(|(_, _, name, _)| *name == "kael moodboard"));
    }
}

#[test]
fn a_reference_turned_all_the_way_down_is_silenced_rather_than_cut() {
    // Two different sentences for the Inspector to say, the same two the text
    // budget distinguishes: "you turned this card down" sends the user to a
    // slider, "this did not fit" sends them upstream. A silenced reference that
    // vanished from the panel instead would read as the attachment having been
    // lost.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let sliders = Sliders::from_pairs([(ashfall.vashk.id, 0.0)]);
    let extracted = ashfall.extract(&world, &sliders);
    let images = compile_images(&extracted, image_budget("gemini-3-pro-image").unwrap());

    assert_eq!(
        ashfall.casualties(images.bucket(RefBucket::Objects).unwrap()),
        vec![(Layer::Ancestry, "Vashk", "vashk silhouette", Silenced),]
    );
    assert_eq!(
        ashfall.casualties(images.bucket(RefBucket::Characters).unwrap()),
        vec![(Layer::Ancestry, "Vashk", "vashk pose", Silenced),]
    );

    // And a silenced reference costs the budget nothing, so turning one card down
    // can never be what takes another card's reference out of the request.
    let tight = ImageBudget {
        objects: Refs::new(1),
        characters: Some(Refs::new(1)),
        style_refs: Some(Refs::new(1)),
    };
    let already_gone: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.contributes()).copied().collect();
    let with_silenced = compile_images(&extracted, tight);
    let without = compile_images(&already_gone, tight);
    for bucket in RefBucket::ALL {
        let (a, b) = (with_silenced.bucket(bucket).unwrap(), without.bucket(bucket).unwrap());
        assert_eq!(ashfall.kept(a), ashfall.kept(b), "{bucket:?}");
    }
}

#[test]
fn a_tie_in_weight_is_broken_towards_the_reference_closest_to_the_subject() {
    // Ties have to break the same way on every machine and in five years, because
    // an `influence_snapshot` has to compile to the same request — "it happened to
    // be a stable sort" is not a contract anybody wrote down. The direction is the
    // one the text budget already drops in: the earlier fragment is the one
    // further out in the stack, and general context is what a subject can most
    // afford to lose.
    let mut vashk = node(NodeKind::Species, "Vashk");
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    kael.links.push(Link::new(vashk.id, LinkRole::SpeciesOf));
    let inherited = wobu_core::new_id();
    let own = wobu_core::new_id();
    vashk.asset_links.push(AssetRef::new(inherited, AssetRole::FullRef));
    kael.asset_links.push(AssetRef::new(own, AssetRole::FullRef));

    let world = World::new(vec![&vashk, &kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
    assert_eq!(extracted[0].weight(), extracted[1].weight(), "the tie this test is about");

    let one =
        ImageBudget { objects: Refs::new(0), characters: None, style_refs: Some(Refs::new(1)) };
    let images = compile_images(&extracted, one);
    let bucket = images.bucket(RefBucket::StyleRefs).unwrap();
    assert_eq!(bucket.kept().iter().map(|f| f.asset_id()).collect::<Vec<_>>(), vec![Some(own)]);
    assert_eq!(
        bucket.dropped().iter().map(|d| (d.fragment.asset_id(), d.reason)).collect::<Vec<_>>(),
        vec![(Some(inherited), Cut)]
    );
}

#[test]
fn the_buckets_do_not_depend_on_the_order_the_nodes_were_loaded() {
    // The "works on my machine" bug this crate is most exposed to, stated on the
    // image channel: the caller hands its nodes over in whatever order its own map
    // iterates in, and a different picture reaches the provider between two
    // launches with nothing in the world to explain it. Held at a cap tight enough
    // that the drop decisions are part of what is being compared.
    let ashfall = Ashfall::new();
    let tight = ImageBudget {
        objects: Refs::new(1),
        characters: Some(Refs::new(1)),
        style_refs: Some(Refs::new(2)),
    };
    let compile_from = |nodes: Vec<&Node>| {
        let world = World::new(nodes);
        let shot = Shot::new("Character sheet · 3:4");
        let stack = resolve(&world, ashfall.kael.id, Some(shot)).unwrap();
        let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
        let images = compile_images(&extracted, tight);
        let kept: Vec<&'static str> = images.kept().map(|f| ashfall.name(&f)).collect();
        let lost: Vec<(&'static str, DropReason)> =
            images.dropped().map(|d| (ashfall.name(&d.fragment), d.reason)).collect();
        format!("{kept:?}|{lost:?}")
    };

    let baseline = compile_from(ashfall.nodes());
    let mut nodes = ashfall.nodes();
    for rotation in 0..nodes.len() {
        nodes.rotate_left(1);
        assert_eq!(compile_from(nodes.clone()), baseline, "rotation {rotation}");
    }
    nodes.reverse();
    assert_eq!(compile_from(nodes), baseline, "reversed");
    assert_eq!(compile_from(ashfall.nodes()), baseline, "and byte-identical twice running");
}

#[test]
fn an_unlimited_budget_keeps_every_reference_and_has_no_denominator_to_print() {
    // `influence_resolve` compiles for display before a backend has been chosen,
    // and a local ComfyUI never gets a cap at all. Both go through this same
    // function rather than round an "unbudgeted" path, because a second path is
    // one on which the report would come to mean something different — and the
    // Inspector shows the same three counters it will show afterwards, without
    // inventing a limit nobody declared.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let images = compile_images(&extracted, ImageBudget::unlimited());

    assert_eq!(
        counters(&images),
        vec![
            (RefBucket::Objects, 2, None),
            (RefBucket::Characters, 2, None),
            (RefBucket::StyleRefs, 5, None),
        ]
    );
    assert_eq!(images.dropped().count(), 0);
    assert_eq!(images.kept().count(), 9, "every reference but the mood board");
}

#[test]
fn a_bucket_that_takes_nothing_drops_everything_and_reports_all_of_it() {
    // There is no floor, unlike the positive prompt, which is trimmed to one
    // fragment and no further: an empty prompt is a picture of nothing that still
    // costs money, whereas a request with no reference images is an ordinary
    // request. What must not happen is that the references disappear quietly, so
    // the whole pool comes back as a drop report the panel can show.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let nothing = ImageBudget {
        objects: Refs::new(0),
        characters: Some(Refs::new(0)),
        style_refs: Some(Refs::new(0)),
    };
    let images = compile_images(&extracted, nothing);

    assert_eq!(images.kept().count(), 0);
    assert_eq!(images.dropped().count(), 9, "every reference but the mood board");
    assert!(images.dropped().all(|d| d.reason == Cut));
}

#[test]
fn the_two_budgets_cannot_take_each_others_material() {
    // The seam between #43 and #44. Backends meter prompt length and reference
    // count separately, so a long description must not be able to cost the user a
    // reference they attached, and a stack rich in references must not shorten the
    // prompt. Stated as two equivalences: each budget produces the same answer
    // over the whole slice as it does over its own half of it.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let images_only: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.asset_id().is_some()).copied().collect();
    let text_only: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.asset_id().is_none()).copied().collect();

    let refs = image_budget("gemini-3-pro-image").unwrap();
    assert_eq!(compile_images(&extracted, refs), compile_images(&images_only, refs));

    let words = Budget { prompt: Chars::new(40), negative: Chars::new(20) };
    assert_eq!(compile(&extracted, words), compile(&text_only, words));
}

#[test]
fn a_thousand_references_are_budgeted_within_the_interactive_budget() {
    // `prompt_compile` runs on every drag of a weight slider
    // (`docs/05-architecture.md`), and this runs beside the text budget on the
    // same slice. A thousand references is far past any real stack; the bound is
    // here to catch a sort that went quadratic, not to measure the machine.
    let mut kael = node(NodeKind::Character, "Kael Vantris");
    for i in 0..1_000 {
        let role = AssetRole::ALL[i % AssetRole::ALL.len()];
        kael.asset_links.push(AssetRef {
            // Many ties, because ties are the case that makes the comparator do
            // the most work.
            weight: (i % 4) as f32 * 0.25,
            ..AssetRef::new(wobu_core::new_id(), role)
        });
    }
    let world = World::new(vec![&kael]);
    let stack = resolve(&world, kael.id, None).unwrap();
    let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());

    // Fastest of several runs, not one — a single sample measures the machine's
    // load as much as this code, and the neighbouring bounds in `prompts.rs` and
    // `stacks.rs` have failed that way with a build running beside them.
    let budget = image_budget("gemini-3-pro-image").unwrap();
    let mut fastest = std::time::Duration::MAX;
    let mut images = compile_images(&extracted, budget);
    for _ in 0..5 {
        let started = std::time::Instant::now();
        images = compile_images(&extracted, budget);
        fastest = fastest.min(started.elapsed());
    }

    assert_eq!(images.kept().count(), 6 + 5 + 3);
    assert!(images.dropped().count() > 0);
    // One millisecond proved narrower than normal CI scheduling noise (1.21 ms
    // on an otherwise green run). Five milliseconds still leaves two orders of
    // magnitude beneath a frame and catches the quadratic regression this test
    // exists to detect without turning runner load into a product failure.
    assert!(fastest < std::time::Duration::from_millis(5), "took {fastest:?}");
}
