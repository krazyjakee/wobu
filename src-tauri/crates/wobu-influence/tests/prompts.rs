//! What actually gets sent, and what the Inspector is told was left behind.
//!
//! Same discipline as `fragments.rs` and for the same reason, one stage further
//! down: a budget that drops the wrong fragment, or an emitter that joins them in
//! the wrong order, throws nothing and looks like nothing — the art just comes
//! out subtly wrong. So the expected prompts and the whole expected drop report
//! are written out longhand, and a diff on one is meant to be read and argued
//! with.

use wobu_core::{
    AssetRef, AssetRole, Description, FragmentTarget, Id, Layer, Link, LinkRole, Node, NodeKind,
    SectionValue, default_preset,
};
use wobu_influence::{
    Budget, Chars, CompiledPrompt, DropReason, Fragment, FragmentBody, Origin, Reached,
    ResolvedSource, Shot, Sliders, World, compile, fragments, resolve,
};

use DropReason::{Budget as Cut, Silenced};

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

/// One line of the drop report: which card, which section, and why.
type Casualty<'a> = (Layer, &'a str, &'static str, DropReason);

fn report<'a>(compiled: &CompiledPrompt<'a>) -> Vec<Casualty<'a>> {
    compiled
        .dropped()
        .iter()
        .map(|d| (d.fragment.layer(), d.fragment.source_name(), d.fragment.section(), d.reason))
        .collect()
}

/// The same three-layer world `fragments.rs` compiles, so the two files can be
/// read side by side: a style guide, a species, a subject, one reference the user
/// is allowed to send and one they are not.
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
        describe(&mut style, [
            ("medium", prose("Oil on board")),
            ("never", list(&["photographic detail"])),
        ]);

        let mut vashk = node(NodeKind::Species, "Vashk");
        describe(&mut vashk, [
            ("silhouette", prose("Long-limbed, four-jointed")),
            ("never", list(&["fur"])),
        ]);

        let pose_ref = wobu_core::new_id();
        let mood_ref = wobu_core::new_id();
        let mut kael = node(NodeKind::Character, "Kael Vantris");
        describe(&mut kael, [
            ("silhouette", prose("Tall, narrow, hooded")),
            ("costume", prose("Ash-grey longcoat")),
            ("palette", list(&["#2b2118", "#c2703a"])),
            ("never", list(&["modern firearms", "clean surfaces"])),
        ]);
        kael.links.push(Link::new(vashk.id, LinkRole::SpeciesOf));
        kael.asset_links.push(AssetRef::new(pose_ref, AssetRole::Pose));
        kael.asset_links.push(AssetRef::new(mood_ref, AssetRole::Mood));

        Ashfall { style, vashk, kael, pose_ref, mood_ref }
    }

    fn nodes(&self) -> Vec<&Node> {
        vec![&self.style, &self.vashk, &self.kael]
    }

    /// Everything the stack contributes, unsorted and unfiltered, which is what
    /// `compile` is specified to be handed.
    fn extract<'a>(&'a self, world: &World<'a>, sliders: &Sliders) -> Vec<Fragment<'a>> {
        let shot = Shot::new("Character sheet · 3:4");
        let stack = resolve(world, self.kael.id, Some(shot)).unwrap();
        fragments(&stack, default_preset(NodeKind::Character), sliders)
    }
}

#[test]
fn a_character_sheet_compiles_layer_by_layer_with_the_subject_last() {
    // The snapshot the rest of the file is written against. General context
    // first, the subject's own specifics after it, the shot's framing last —
    // which is where a text encoder's recency bias does the most good, and is
    // the whole reason the layer order is the product rather than an
    // implementation detail. The negative prompt is every layer's `never`
    // section in the same order.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let compiled = compile(&extracted, Budget::unlimited());

    assert_eq!(
        compiled.prompt(),
        format!(
            "Oil on board, Long-limbed, four-jointed, Tall, narrow, hooded, Ash-grey longcoat, \
             #2b2118, #c2703a, {}",
            default_preset(NodeKind::Character).framing
        )
    );
    assert_eq!(compiled.negative(), "photographic detail, fur, modern firearms, clean surfaces");
    assert_eq!(report(&compiled), vec![]);
    assert_eq!(compiled.overflow(), None);
}

#[test]
fn dropping_is_by_weight_and_emitting_is_by_layer() {
    // The bug this test exists for: reusing the weight-sorted list to build the
    // prompt. It compiles, it fits the budget, and it reads specific-to-general —
    // so the encoder's recency bias lands on the style guide instead of the
    // subject, forever, with nothing on screen to explain it.
    //
    // A budget that fits three of the seven prompt fragments. The two heaviest
    // are both `silhouette`, which a character sheet leans on hardest, and one of
    // them belongs to the *outermost* card in the stack — so weight and layer
    // disagree here, which is what makes the emitted order worth asserting.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let compiled =
        compile(&extracted, Budget { prompt: Chars::new(70), negative: Chars::UNLIMITED });

    // Ancestry (1.4) survives while the subject's own palette (1.0) does not, so
    // weight decided who went — and the survivor still emits ahead of the
    // subject, so layer decided the order.
    assert_eq!(
        compiled.prompt(),
        "Long-limbed, four-jointed, Tall, narrow, hooded, Ash-grey longcoat"
    );
    assert_eq!(report(&compiled), vec![
        (Layer::Style, "Ashfall House Style", "medium", Cut),
        (Layer::Subject, "Kael Vantris", "palette", Cut),
        (Layer::Subject, "Kael Vantris", "palette", Cut),
        (Layer::Shot, "Character sheet · 3:4", "framing", Cut),
    ]);
    // The negatives were nowhere near their own budget and are untouched.
    assert_eq!(compiled.negative(), "photographic detail, fur, modern firearms, clean surfaces");
    assert_eq!(compiled.overflow(), None);
}

#[test]
fn ties_in_weight_are_broken_furthest_from_the_subject_first() {
    // Four `never` items, all at 1.0, and only two fit. Which two is not allowed
    // to be whatever the sort happened to do: an `influence_snapshot` has to
    // compile to the same prompt in five years. General context is what a subject
    // can most afford to lose, so the outermost cards go first.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let compiled =
        compile(&extracted, Budget { prompt: Chars::UNLIMITED, negative: Chars::new(33) });

    assert_eq!(compiled.negative(), "modern firearms, clean surfaces");
    assert_eq!(report(&compiled), vec![
        (Layer::Style, "Ashfall House Style", "never", Cut),
        (Layer::Ancestry, "Vashk", "never", Cut),
    ]);
}

#[test]
fn the_two_prompts_are_budgeted_against_separate_pools() {
    // They are separate fields of every request this app will build, and metered
    // separately. Charging the negatives against the positive prompt's limit
    // would drop the subject's costume to make room for a `never` list that was
    // never competing for that space.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());

    let starved_negative =
        compile(&extracted, Budget { prompt: Chars::UNLIMITED, negative: Chars::new(17) });
    assert!(starved_negative.prompt().contains("Ash-grey longcoat"), "the positive is untouched");
    // Not the shortest of the four — they all weigh the same, so what survives
    // is the one nearest the subject. Length is what a fragment costs, never
    // what decides whether it is the one to go.
    assert_eq!(starved_negative.negative(), "clean surfaces");

    let starved_prompt =
        compile(&extracted, Budget { prompt: Chars::new(25), negative: Chars::UNLIMITED });
    assert_eq!(starved_prompt.prompt(), "Tall, narrow, hooded");
    assert_eq!(
        starved_prompt.negative(),
        "photographic detail, fur, modern firearms, clean surfaces",
        "the negatives are untouched"
    );
}

#[test]
fn a_budget_too_small_for_one_fragment_keeps_the_strongest_and_says_so() {
    // The impossible case, which is real: a preset's framing text alone can
    // outrun a tight limit. An empty positive prompt is not a smaller prompt, it
    // is a different picture — the backend renders whatever the negatives and the
    // seed suggest and bills for it — so the budget is allowed to be wrong out
    // loud instead of silently right. The negative prompt has no such problem:
    // an empty one is a legitimate request, so it is emptied rather than overrun.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let compiled = compile(&extracted, Budget { prompt: Chars::new(5), negative: Chars::new(5) });

    // The last survivor is the heaviest, and the tie between the two 1.4s broke
    // towards the subject's own — which is the one worth keeping when only one
    // fragment of the whole stack is going to be sent.
    assert_eq!(compiled.prompt(), "Tall, narrow, hooded");
    assert_eq!(compiled.overflow(), Some(Chars::new(15)));
    assert_eq!(compiled.negative(), "");

    // And every single casualty is accounted for by card and section, rather
    // than the string having been cut off somewhere in the middle.
    assert_eq!(report(&compiled), vec![
        (Layer::Style, "Ashfall House Style", "medium", Cut),
        (Layer::Style, "Ashfall House Style", "never", Cut),
        (Layer::Ancestry, "Vashk", "silhouette", Cut),
        (Layer::Ancestry, "Vashk", "never", Cut),
        (Layer::Subject, "Kael Vantris", "costume", Cut),
        (Layer::Subject, "Kael Vantris", "palette", Cut),
        (Layer::Subject, "Kael Vantris", "palette", Cut),
        (Layer::Subject, "Kael Vantris", "never", Cut),
        (Layer::Subject, "Kael Vantris", "never", Cut),
        (Layer::Shot, "Character sheet · 3:4", "framing", Cut),
    ]);
}

#[test]
fn an_empty_positive_prompt_is_never_the_budgets_doing() {
    // The invariant #46 gets to lean on when it decides whether to send: nothing
    // came out because nothing went in, or something came out and `overflow` says
    // whether it fits. There is no third state in which the budget quietly
    // emptied a prompt that then went to a provider.
    let nothing = compile(&[], Budget { prompt: Chars::new(0), negative: Chars::new(0) });
    assert_eq!(nothing.prompt(), "");
    assert_eq!(nothing.negative(), "");
    assert_eq!(nothing.overflow(), None, "an empty prompt out of an empty stack is not an overrun");

    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let starved = compile(&extracted, Budget { prompt: Chars::new(0), negative: Chars::new(0) });
    assert!(!starved.prompt().is_empty());
    assert!(starved.overflow().is_some());
}

#[test]
fn a_mood_reference_is_in_neither_prompt_and_is_not_a_casualty() {
    // `wobu-core` proved it at the link layer and #42 proved the fragment layer
    // did not lose it; this proves the thing that actually assembles a request
    // did not either. It is also deliberately absent from the drop report: a mood
    // reference is doing exactly what it was attached to do, and calling it a
    // casualty would send the user off to fix something that is not broken.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    assert!(
        extracted.iter().any(|f| f.asset_id() == Some(ashfall.mood_ref)),
        "the fixture must offer one for this to prove anything"
    );

    // Held at a budget so tight that all but one fragment of the stack is cut,
    // because "everything was dropped anyway" is the state in which a mood
    // reference could most plausibly slip into the report as one more casualty.
    let compiled = compile(&extracted, Budget { prompt: Chars::new(5), negative: Chars::new(5) });
    for dropped in compiled.dropped() {
        assert_ne!(dropped.fragment.asset_id(), Some(ashfall.mood_ref), "{dropped:?}");
    }
    assert!(!compiled.prompt().is_empty(), "and something survived, so this is not vacuous");
    let mood = ashfall.mood_ref.to_string();
    assert!(!compiled.prompt().contains(&mood));
    assert!(!compiled.negative().contains(&mood));
}

#[test]
fn text_that_may_not_leave_the_machine_is_not_prompt_material_at_any_budget() {
    // The same privacy property stated over text rather than over an image.
    // Nothing routes prose to `moodboard_only` today — `section_target` only ever
    // answers `prompt` or `negative` — so this is built by hand on purpose: the
    // filter is `Fragment::is_sendable` and nothing else, exactly as `wobu-core`
    // and #42 specify, so a section that starts routing that way is held back
    // here without anybody having to remember to come and add it.
    //
    // The budget fits `alpha` and `beta` exactly, separators included. If the
    // private text were priced at all, one of the two would have to go, so this
    // asserts both halves at once: it is not in the prompt, and it did not cost
    // anything that is.
    let source = ResolvedSource {
        layer: Layer::Subject,
        origin: Origin::Shot("Character sheet"),
        reached: Reached::Shot,
        distance: 0,
        weight: 1.0,
    };
    let private = "x".repeat(500);
    let hand_built = [
        Fragment::new(&source, "medium", FragmentBody::Text("alpha"), 1.0, FragmentTarget::Prompt),
        Fragment::new(
            &source,
            "medium",
            FragmentBody::Text(&private),
            1.0,
            FragmentTarget::MoodboardOnly,
        ),
        Fragment::new(&source, "medium", FragmentBody::Text("beta"), 1.0, FragmentTarget::Prompt),
    ];

    let compiled =
        compile(&hand_built, Budget { prompt: Chars::new(13), negative: Chars::UNLIMITED });
    assert_eq!(compiled.prompt(), "alpha, beta");
    assert_eq!(compiled.negative(), "");
    assert_eq!(report(&compiled), vec![], "it was never in the running to be dropped");
    assert!(!compiled.prompt().contains('x'));
}

#[test]
fn a_reference_image_is_not_text_and_the_text_budget_does_not_price_it() {
    // Images are not characters, so a text budget prices them at nothing rather
    // than at a guess — and it makes no decision about them at all, which is why
    // they are not in this drop report either. Their budget is the per-role one
    // against the backend's declared capability (`docs/08-providers.md`), it is
    // the tighter of the two, and it is #44.
    //
    // Stated as an equivalence rather than as an arithmetic assertion, so it
    // survives an edit to the fixture's prose: compiling with the references and
    // compiling with them already removed must produce the same answer, down to
    // the drop report.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let extracted = ashfall.extract(&world, &Sliders::neutral());
    let text_only: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.asset_id().is_none()).copied().collect();
    assert_eq!(extracted.len(), text_only.len() + 2, "a pose reference and a mood reference");

    let tight = Budget { prompt: Chars::new(70), negative: Chars::new(33) };
    assert_eq!(compile(&extracted, tight), compile(&text_only, tight));

    // And the conditioning reference, which is very much going to be sent, is
    // still not a word in the prompt.
    let compiled = compile(&extracted, Budget::unlimited());
    assert!(!compiled.prompt().contains(&ashfall.pose_ref.to_string()));
    assert!(!compiled.negative().contains(&ashfall.pose_ref.to_string()));
}

#[test]
fn a_layer_turned_all_the_way_down_is_reported_as_silenced_rather_than_cut() {
    // Two different sentences for the Inspector to say, which is why the report
    // is an enum and not a bool: "you turned this card down" sends the user to a
    // slider, "this did not fit" sends them upstream to write leaner notes.
    let ashfall = Ashfall::new();
    let world = World::new(ashfall.nodes());
    let sliders = Sliders::from_pairs([(ashfall.vashk.id, 0.0)]);
    let extracted = ashfall.extract(&world, &sliders);
    let compiled = compile(&extracted, Budget::unlimited());

    assert_eq!(report(&compiled), vec![
        (Layer::Ancestry, "Vashk", "silhouette", Silenced),
        (Layer::Ancestry, "Vashk", "never", Silenced),
    ]);
    assert!(!compiled.prompt().contains("Long-limbed"));
    assert!(!compiled.negative().contains("fur"));

    // And a silenced fragment costs the budget nothing, so turning one card down
    // can never be what takes another card's fragment out.
    let tight = Budget { prompt: Chars::new(70), negative: Chars::new(33) };
    let already_gone: Vec<Fragment<'_>> =
        extracted.iter().filter(|f| f.contributes()).copied().collect();
    assert_eq!(compile(&extracted, tight).prompt(), compile(&already_gone, tight).prompt());
    assert_eq!(compile(&extracted, tight).negative(), compile(&already_gone, tight).negative());
}

#[test]
fn the_compiled_prompt_does_not_depend_on_the_order_the_nodes_were_loaded() {
    // The "works on my machine" bug this crate is most exposed to, stated at the
    // last stage: the caller hands its nodes over in whatever order its own map
    // iterates in, and the prompt that reaches a provider differs between two
    // launches with nothing in the world to explain it. Held at a budget tight
    // enough that the drop decisions are part of what is being compared.
    let ashfall = Ashfall::new();
    let tight = Budget { prompt: Chars::new(70), negative: Chars::new(33) };
    let compile_from = |nodes: Vec<&Node>| {
        let world = World::new(nodes);
        let shot = Shot::new("Character sheet · 3:4");
        let stack = resolve(&world, ashfall.kael.id, Some(shot)).unwrap();
        let extracted = fragments(&stack, default_preset(NodeKind::Character), &Sliders::neutral());
        let compiled = compile(&extracted, tight);
        format!("{:?}|{:?}|{:?}", compiled.prompt(), compiled.negative(), report(&compiled))
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
fn an_absurd_number_of_fragments_compiles_in_well_under_a_millisecond() {
    // `prompt_compile` runs on every Inspector interaction — every drag of a
    // weight slider — so this is a product requirement rather than a nicety
    // (`docs/05-architecture.md`), and it is the same bound `stacks.rs` and
    // `fragments.rs` hold the two stages before this one to. The budget is set to
    // bite, because the sort and the drop loop are the part that could be slow.
    //
    // Eight hundred prompt fragments is what `fragments.rs`' thousand-node world
    // hands over — eight on each of a hundred layer cards, which is already an
    // order of magnitude past anything the data model can really produce. They
    // are hand-built rather than resolved because what is being timed here is the
    // sort, the drop loop and the join, and none of the three can tell where a
    // fragment came from.
    let source = ResolvedSource {
        layer: Layer::Subject,
        origin: Origin::Shot("Character sheet"),
        reached: Reached::Shot,
        distance: 0,
        weight: 1.0,
    };
    let texts: Vec<String> = (0..800).map(|i| format!("Ash-grey longcoat number {i}")).collect();
    let built: Vec<Fragment<'_>> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            Fragment::new(
                &source,
                "costume",
                FragmentBody::Text(text),
                // Many ties, because ties are the case that makes the comparator
                // do the most work.
                (i % 4) as f32 * 0.5,
                FragmentTarget::Prompt,
            )
        })
        .collect();

    let started = std::time::Instant::now();
    let compiled = compile(&built, Budget { prompt: Chars::new(2_000), negative: Chars::new(0) });
    let elapsed = started.elapsed();

    assert!(compiled.prompt().chars().count() <= 2_000);
    assert!(!compiled.dropped().is_empty());
    assert!(elapsed < std::time::Duration::from_millis(1), "took {elapsed:?}");
}
