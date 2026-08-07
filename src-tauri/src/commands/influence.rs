//! The Inspector's two read-only questions about the influence engine.
//!
//! `influence_resolve` answers "which layers reached this subject", and
//! `prompt_compile` answers "what prompt do they compile to, and what was left
//! out". Both answer from the local index and are pure after that, because the
//! panel calls them on every slider drag.

use tauri::State;
use wobu_core::{FragmentTarget, Id, Layer, Node, NodeKind, Preset, default_preset};
use wobu_influence::{
    Budget, Chars, DropReason, Dropped, Fragment, FragmentBody, Reached, ResolvedStack, Shot,
    Sliders, World, compile, fragments, resolve,
};

use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

/// Where one layer card's weight slider sits.
///
/// A list of pairs rather than an object keyed by node id. Both deserialize, but
/// a key that is not a ULID would be dropped in silence, and a slider that
/// quietly applies to no card is indistinguishable from an engine ignoring the
/// user.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SliderSetting {
    node_id: Id,
    value: f32,
    #[serde(default)]
    muted: bool,
}

/// The Shot layer, as the Inspector's controls describe it.
///
/// Both fields optional, because layer 7 is the one the panel owns and the panel
/// is #47. `label` is only what the card is titled — the framing text itself
/// comes from the preset — so it defaults to the preset's label, which is what
/// the card would have said anyway.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotControls {
    label: Option<String>,
    weight: Option<f32>,
    /// Extra framing typed for this run. Separate from `label`, which only
    /// names the card and is never sent to a provider.
    prompt: Option<String>,
}

/// What one compilation may spend on text.
///
/// Characters, not tokens: there is no tokenizer in this workspace and
/// deliberately will not be one (`wobu_influence::Chars`). Either limit absent
/// means unlimited, because no backend has been chosen yet — `Capabilities`
/// (#50) is what will state a real one, and inventing a number here would drop
/// fragments to fit a limit nobody has measured.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBudget {
    prompt_chars: Option<usize>,
    negative_chars: Option<usize>,
}

/// One thing one layer contributes, with everything needed to point at where it
/// came from.
///
/// One shape for all three lists — a card's contributions, the spans in the
/// compiled prompt, and the drop report — because they are the same fragments
/// seen from three angles, and the panel draws the same row for each. The
/// alternative was three near-identical interfaces that would drift apart the
/// first time one gained a field.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceFragment {
    layer: Layer,
    /// Null for the Shot layer, whose framing text comes from the preset rather
    /// than from any node.
    node_id: Option<Id>,
    source_name: String,
    section: &'static str,
    /// Prose. Null for a reference image, which carries `assetId` instead —
    /// exactly one of the two is ever set.
    text: Option<String>,
    asset_id: Option<Id>,
    /// `link.weight × section_priority × user_slider`, already multiplied out.
    weight: f32,
    target: FragmentTarget,
    /// Whether this may be put in front of a provider. False only for
    /// `moodboard_only`, and read from the engine rather than re-derived from
    /// the target here: two lists of what is private would be one rename away
    /// from disagreeing, and that disagreement fails in the direction of
    /// somebody's mood board on a third party's servers.
    sendable: bool,
}

/// One layer card.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerCard {
    layer: Layer,
    node_id: Option<Id>,
    /// What the card is titled — the node's name, or the shot's label.
    name: String,
    kind: Option<NodeKind>,
    reached: Reached,
    /// Hops from whichever root reached this source. The subject and the two
    /// seeded singletons are 0.
    distance: u16,
    /// The product of the link weights along the path that reached this source.
    /// Kept apart from `slider` so the panel can show what each contributed.
    weight: f32,
    slider: f32,
    fragments: Vec<InfluenceFragment>,
}

/// The resolved stack for one subject.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfluenceStack {
    subject_id: Id,
    /// The preset this was resolved under — the caller's, or the kind's default
    /// when it named none. Returned whole rather than as an id because the panel
    /// needs its aspect and image count to describe what Generate would do, and
    /// a second round trip for a `&'static` table would be a round trip for
    /// nothing.
    preset: &'static Preset,
    layers: Vec<LayerCard>,
}

/// One fragment the compiler left out, and why.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFragment {
    fragment: InfluenceFragment,
    reason: DropReason,
}

/// The two prompt strings, and the account of everything that is not in them.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledPrompt {
    subject_id: Id,
    preset: &'static Preset,
    prompt: String,
    negative: String,
    /// The fragments that are in the two strings above, in emission order. This
    /// is what lets the compiled-prompt box tint each span by origin, which
    /// `docs/04-influence-engine.md` calls the main feedback loop for learning to
    /// write good upstream notes rather than a debug feature.
    spans: Vec<InfluenceFragment>,
    /// Everything left out, in reading order, so the panel can walk it alongside
    /// the layer cards. Present because "the Inspector reports what was dropped
    /// rather than truncating silently" — a command that returned only the
    /// prompt would throw away the thing that makes the panel worth having.
    dropped: Vec<DroppedFragment>,
    /// How far over its budget the positive prompt is, or null when it fits.
    /// Only ever set when the budget could not fit even one fragment: the
    /// compiler keeps the heaviest and says so, because an empty prompt is not a
    /// smaller picture, it is a different one that still costs money.
    overflow: Option<usize>,
}

/// The resolved stack for a subject, with the per-layer detail the Inspector's
/// layer cards read.
///
/// Answers from the local index and never touches the project folder, so this is
/// as fast on a share that is currently unplugged as on an SSD — see
/// `Project::world_nodes`. A project with no Style Guide, or none of the links
/// the stack walks, resolves to a short list rather than an error; that is the
/// state every project is in on day one and the panel is on screen for all of it.
#[tauri::command]
pub fn influence_resolve(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<SliderSetting>>,
    shot: Option<ShotControls>,
) -> CommandResult<InfluenceStack> {
    let sliders = sliders_from(sliders);
    state.with(|p| {
        resolved(p.world_nodes()?, subject_id, preset.as_deref(), &sliders, shot.as_ref())
    })
}

/// The compiled positive and negative prompt, the spans they are made of, and
/// the account of what did not make it.
///
/// Called on every Inspector interaction — every slider drag, every preset
/// change — so it does no IO at all: the world comes out of the local index and
/// the engine itself is pure.
#[tauri::command]
pub fn prompt_compile(
    state: State<'_, AppState>,
    subject_id: Id,
    preset: Option<String>,
    sliders: Option<Vec<SliderSetting>>,
    shot: Option<ShotControls>,
    budget: Option<PromptBudget>,
) -> CommandResult<CompiledPrompt> {
    let sliders = sliders_from(sliders);
    let budget = budget_from(budget);
    state.with(|p| {
        compiled(p.world_nodes()?, subject_id, preset.as_deref(), &sliders, shot.as_ref(), budget)
    })
}

/// Everything [`influence_resolve`] does once it has the nodes.
///
/// Separated from the command so that the tests below exercise the payload the
/// webview actually receives rather than a re-assembly of it — the bridge
/// contract is the thing worth pinning, and a test that built its own struct
/// would agree with itself no matter what the command sent.
fn resolved<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    preset: Option<&str>,
    sliders: &Sliders,
    shot: Option<&'a ShotControls>,
) -> CommandResult<InfluenceStack> {
    let sheet = preset_for(nodes, subject_id, preset)?;
    let user_prompt = shot.and_then(|controls| controls.prompt.as_deref());
    // No Shot layer unless the caller named one: resolving for display is not
    // resolving for a generation, and a card invented here would put framing on
    // screen for a shot nobody has set up (`wobu_influence::Shot`).
    let shot = shot.map(|controls| Shot {
        label: controls.label.as_deref().unwrap_or("Shot"),
        weight: controls.weight.unwrap_or(1.0),
    });
    let (stack, mut extracted) = prepare(nodes, subject_id, sheet, sliders, shot)?;
    append_shot_prompt(&stack, &mut extracted, user_prompt);
    Ok(InfluenceStack {
        subject_id,
        preset: sheet,
        layers: layer_cards(&stack, &extracted, sliders),
    })
}

/// Everything [`prompt_compile`] does once it has the nodes.
fn compiled<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    preset: Option<&str>,
    sliders: &Sliders,
    shot: Option<&'a ShotControls>,
    budget: Budget,
) -> CommandResult<CompiledPrompt> {
    let sheet = preset_for(nodes, subject_id, preset)?;
    let user_prompt = shot.and_then(|controls| controls.prompt.as_deref());
    // Always a shot, unlike `resolved`. The preset's framing text is the Shot
    // layer's whole contribution, so a prompt compiled without one would differ
    // from the prompt a generation actually sends — which is the single thing
    // this panel must never be wrong about.
    let shot = Shot {
        label: shot.and_then(|c| c.label.as_deref()).unwrap_or(sheet.label),
        weight: shot.and_then(|c| c.weight).unwrap_or(1.0),
    };
    let (stack, mut extracted) = prepare(nodes, subject_id, sheet, sliders, Some(shot))?;
    append_shot_prompt(&stack, &mut extracted, user_prompt);

    let compiled = compile(&extracted, budget);
    Ok(CompiledPrompt {
        subject_id,
        preset: sheet,
        prompt: compiled.prompt().to_owned(),
        negative: compiled.negative().to_owned(),
        spans: prompt_spans(&extracted, compiled.dropped()),
        dropped: compiled
            .dropped()
            .iter()
            .map(|d| DroppedFragment { fragment: fragment_view(&d.fragment), reason: d.reason })
            .collect(),
        overflow: compiled.overflow().map(Chars::get),
    })
}

/// Resolve the stack and extract its fragments — the half both commands share.
fn prepare<'a>(
    nodes: &'a [Node],
    subject_id: Id,
    sheet: &Preset,
    sliders: &Sliders,
    shot: Option<Shot<'a>>,
) -> CommandResult<(ResolvedStack<'a>, Vec<Fragment<'a>>)> {
    let world = World::new(nodes.iter());
    // `resolve` is `None` only for a subject outside the view, which
    // `preset_for` has already ruled out at both call sites; restated rather
    // than unwrapped because a panic here would take the window with it.
    let stack = resolve(&world, subject_id, shot).ok_or_else(|| no_such_subject(subject_id))?;
    let extracted = fragments(&stack, sheet, sliders);
    Ok((stack, extracted))
}

fn append_shot_prompt<'a>(
    stack: &ResolvedStack<'a>,
    extracted: &mut Vec<Fragment<'a>>,
    prompt: Option<&'a str>,
) {
    let Some(prompt) = prompt.map(str::trim).filter(|prompt| !prompt.is_empty()) else {
        return;
    };
    if let Some(source) = stack.sources().iter().find(|source| source.layer == Layer::Shot) {
        extracted.push(Fragment::new(
            source,
            "user_prompt",
            FragmentBody::Text(prompt),
            source.weight,
            FragmentTarget::Prompt,
        ));
    }
}

/// The preset a compilation runs under, and the check that the subject exists.
///
/// The two together because the default preset is a property of the subject's
/// kind, so there is no answering the first without the second.
///
/// A preset id the registry has never heard of falls back to the kind's default
/// rather than failing. `Generation.preset` is a string that outlives any one
/// build (`wobu-core`'s `preset.rs`), so a snapshot naming a preset since
/// renamed must still compile to something rather than take the panel down.
fn preset_for(
    nodes: &[Node],
    subject_id: Id,
    preset: Option<&str>,
) -> CommandResult<&'static Preset> {
    let subject =
        nodes.iter().find(|n| n.id == subject_id).ok_or_else(|| no_such_subject(subject_id))?;
    Ok(preset.and_then(wobu_core::preset).unwrap_or_else(|| default_preset(subject.kind)))
}

/// A subject that is not in the world.
///
/// The ordinary cause is a tab or an Inspector still pointing at something a
/// collaborator deleted, which is why it is `node.not_found` and not an internal
/// error: the frontend already knows what to do with that code.
fn no_such_subject(id: Id) -> WobuError {
    WobuError::new(Code::NoSuchNode, "That entity is not in this project any more.")
        .with_detail(id.to_string())
}

fn sliders_from(settings: Option<Vec<SliderSetting>>) -> Sliders {
    Sliders::from_pairs(
        settings
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.node_id, if s.muted { 0.0 } else { s.value })),
    )
}

fn budget_from(budget: Option<PromptBudget>) -> Budget {
    let Some(budget) = budget else { return Budget::unlimited() };
    Budget {
        prompt: budget.prompt_chars.map_or(Chars::UNLIMITED, Chars::new),
        negative: budget.negative_chars.map_or(Chars::UNLIMITED, Chars::new),
    }
}

fn layer_cards<'a>(
    stack: &ResolvedStack<'a>,
    extracted: &[Fragment<'a>],
    sliders: &Sliders,
) -> Vec<LayerCard> {
    stack
        .sources()
        .iter()
        .map(|source| LayerCard {
            layer: source.layer,
            node_id: source.node_id(),
            name: source.name().to_owned(),
            kind: source.node().map(|n| n.kind),
            reached: source.reached,
            distance: source.distance,
            weight: source.weight,
            slider: sliders.for_source(source),
            // Grouped on layer and node because resolution visits each node
            // exactly once — first visit wins — so the pair names one card and
            // never two. Extraction does emit in source order, but grouping on
            // that would make the cards depend on an ordering no type states.
            fragments: extracted
                .iter()
                .filter(|f| f.layer() == source.layer && f.node_id() == source.node_id())
                .map(fragment_view)
                .collect(),
        })
        .collect()
}

fn fragment_view(fragment: &Fragment<'_>) -> InfluenceFragment {
    InfluenceFragment {
        layer: fragment.layer(),
        node_id: fragment.node_id(),
        source_name: fragment.source_name().to_owned(),
        section: fragment.section(),
        text: fragment.text().map(str::to_owned),
        asset_id: fragment.asset_id(),
        weight: fragment.weight(),
        target: fragment.target(),
        sendable: fragment.is_sendable(),
    }
}

/// The fragments that are actually in the two compiled strings, in the order
/// they were emitted.
///
/// Derived from the drop report rather than re-decided: by `compile`'s own
/// account, a text fragment that is sendable and not in `dropped` is in one of
/// the two prompts. Working it out that way keeps one compiler in the workspace
/// — a second opinion here about what fits would disagree with the first the
/// moment either changed, and the symptom would be a prompt box highlighting
/// spans that are not in the prompt.
///
/// A `moodboard_only` fragment is in neither list and so appears in neither: it
/// is not sendable, and `compile` deliberately does not report it as a casualty.
///
/// The report is a subsequence of `extracted` in reading order, so one cursor
/// over each is enough. Two fragments that compare equal are identical in every
/// field a caller can see, which is why crediting the wrong one of a pair cannot
/// change the answer.
fn prompt_spans<'a>(extracted: &[Fragment<'a>], dropped: &[Dropped<'a>]) -> Vec<InfluenceFragment> {
    let mut cut = dropped.iter().peekable();
    let mut out = Vec::new();
    for fragment in extracted {
        if cut.peek().is_some_and(|d| d.fragment == *fragment) {
            cut.next();
            continue;
        }
        if fragment.text().is_some() && fragment.is_sendable() {
            out.push(fragment_view(fragment));
        }
    }
    out
}

/* ── conflicts ────────────────────────────────────────────────────────────── */

/// The two influence commands, over a world built here rather than on disk.
///
/// Everything below calls [`resolved`] and [`compiled`] — the exact functions
/// the commands call once they have the nodes — so what is asserted is the
/// payload the webview receives. Where the project folder and the index come
/// into it is `wobu-store`'s `world.rs`, which is the other half of this.
#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::AssetRole;
    use wobu_core::asset::AssetRef;
    use wobu_core::{Description, Link, LinkRole, SectionValue};

    /// A style guide, a culture the subject belongs to, and the subject — the
    /// smallest world with more than one layer in it.
    struct Ashfall {
        nodes: Vec<Node>,
        kael: Id,
        mood: Id,
    }

    fn ashfall() -> Ashfall {
        let mut style = Node::new(NodeKind::StyleGuide, "Ashfall House Style").unwrap();
        style.description = Some(Description::from_sections([(
            "rendering".to_string(),
            SectionValue::Text("Ash-dusted, matte, hand-painted".into()),
        )]));

        let mut guild = Node::new(NodeKind::Culture, "Cinder Guild").unwrap();
        guild.description = Some(Description::from_sections([(
            "costume".to_string(),
            SectionValue::Text("Ash-grey longcoats, brass fastenings".into()),
        )]));

        let mut kael = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
        kael.links = vec![Link::new(guild.id, LinkRole::MemberOf)];
        kael.description = Some(Description::from_sections([
            ("silhouette".to_string(), SectionValue::Text("Tall, narrow, hooded".into())),
            (
                "never".to_string(),
                SectionValue::List(vec!["modern firearms".into(), "neon".into()]),
            ),
        ]));

        // One reference the compiler may send and one it may not. The mood board
        // is the whole reason the second exists.
        let mood = wobu_core::new_id();
        kael.asset_links = vec![
            AssetRef::new(wobu_core::new_id(), AssetRole::Palette),
            AssetRef::new(mood, AssetRole::Mood),
        ];

        let (kael_id, nodes) = (kael.id, vec![style, guild, kael]);
        Ashfall { nodes, kael: kael_id, mood }
    }

    fn stack(world: &Ashfall) -> InfluenceStack {
        resolved(&world.nodes, world.kael, None, &Sliders::neutral(), None).unwrap()
    }

    fn prompt(world: &Ashfall) -> CompiledPrompt {
        compiled(&world.nodes, world.kael, None, &Sliders::neutral(), None, Budget::unlimited())
            .unwrap()
    }

    #[test]
    fn a_layer_card_matches_the_layercard_interface() {
        // Hand-written TypeScript on the far side, so a serde rename nothing
        // noticed arrives in the panel as `undefined` rather than as an error.
        let json = serde_json::to_value(stack(&ashfall())).unwrap();

        for key in ["subjectId", "preset", "layers"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from InfluenceStack");
        }
        let card = &json["layers"][0];
        for key in [
            "layer",
            "nodeId",
            "name",
            "kind",
            "reached",
            "distance",
            "weight",
            "slider",
            "fragments",
        ] {
            assert!(card.get(key).is_some(), "`{key}` is missing from LayerCard");
        }
        // Layer 1 first, and the enums in the snake_case the unions in `api.ts`
        // are written in.
        assert_eq!(card["layer"], "style");
        assert_eq!(card["kind"], "style_guide");
        assert_eq!(card["reached"], "root");
        assert_eq!(json["preset"]["id"], "character_sheet");
    }

    #[test]
    fn a_fragment_matches_the_influencefragment_interface() {
        let json = serde_json::to_value(stack(&ashfall())).unwrap();
        let fragment = &json["layers"][0]["fragments"][0];

        for key in [
            "layer",
            "nodeId",
            "sourceName",
            "section",
            "text",
            "assetId",
            "weight",
            "target",
            "sendable",
        ] {
            assert!(fragment.get(key).is_some(), "`{key}` is missing from InfluenceFragment");
        }
        assert_eq!(fragment["section"], "rendering");
        assert_eq!(fragment["target"], "prompt");
        // Prose and picture are exclusive, and the unused one is `null` rather
        // than an absent key — `text: string | null` on the far side.
        assert!(fragment["assetId"].is_null());
    }

    #[test]
    fn a_compiled_prompt_matches_the_compiledprompt_interface() {
        let world = ashfall();
        let cramped = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            None,
            Budget { prompt: Chars::new(40), negative: Chars::UNLIMITED },
        )
        .unwrap();
        let json = serde_json::to_value(cramped).unwrap();

        for key in ["subjectId", "preset", "prompt", "negative", "spans", "dropped", "overflow"] {
            assert!(json.get(key).is_some(), "`{key}` is missing from CompiledPrompt");
        }
        // The drop report nests the fragment exactly as `wobu_influence::Dropped`
        // does, so the panel renders a casualty with the same component it
        // renders a span with.
        let dropped = &json["dropped"][0];
        assert!(dropped["fragment"].get("sourceName").is_some());
        assert!(["silenced", "budget"].contains(&dropped["reason"].as_str().unwrap()));
        // A prompt that fits reports no overflow — `number | null`, not absent.
        assert!(serde_json::to_value(prompt(&world)).unwrap()["overflow"].is_null());
    }

    #[test]
    fn a_mood_reference_reaches_the_layer_card_and_nothing_a_backend_would_see() {
        // The privacy property at the bridge, which is the last place it can be
        // lost. #26, #42 and #43 each preserved it; a command that put the whole
        // fragment list in its response would undo all three, and the failure
        // would be somebody's mood board arriving at a third party.
        let world = ashfall();
        let cards = serde_json::to_value(stack(&world)).unwrap();
        let mood = world.mood.to_string();

        // Visible to the panel, which is the point of attaching it: the card
        // counts it, and says it must not be sent.
        let fragments = cards["layers"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["fragments"].as_array().unwrap().iter());
        let shown = fragments
            .filter(|f| f["assetId"] == mood)
            .inspect(|f| {
                assert_eq!(f["target"], "moodboard_only");
                assert_eq!(f["sendable"], false);
            })
            .count();
        assert_eq!(shown, 1, "the mood reference should be on its layer card");

        // And absent from every part of what a generation would be built from —
        // asserted over the whole serialised payload rather than field by field,
        // so a field added later is covered without anyone remembering to.
        let compiled = serde_json::to_string(&prompt(&world)).unwrap();
        assert!(!compiled.contains(&mood), "the mood reference crossed the bridge in {compiled}");
        assert!(!compiled.contains("moodboard_only"), "got {compiled}");
    }

    #[test]
    fn the_spans_are_exactly_what_is_in_the_two_prompts() {
        // The attribution trail has to describe the string beside it, or the
        // tinting points at the wrong words. Derived from the drop report rather
        // than re-compiled (`prompt_spans`), so this is what catches the two
        // going out of step.
        let world = ashfall();
        let compiled = prompt(&world);
        let joined = |target: &str| {
            compiled
                .spans
                .iter()
                .filter(|s| serde_json::to_value(s.target).unwrap() == target)
                .map(|s| s.text.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ")
        };

        assert_eq!(joined("prompt"), compiled.prompt);
        assert_eq!(joined("negative"), compiled.negative);
        assert!(compiled.negative.contains("modern firearms"), "got {}", compiled.negative);
        // The subject reads last, ahead of only the framing, where a text
        // encoder's recency bias does the most good.
        assert!(compiled.prompt.starts_with("Ash-dusted"), "got {}", compiled.prompt);
        assert!(compiled.prompt.ends_with("single subject"), "got {}", compiled.prompt);
    }

    #[test]
    fn a_subject_nobody_has_heard_of_is_a_missing_node_rather_than_a_panic() {
        // The ordinary cause is a tab still pointing at something a collaborator
        // deleted, which the frontend already knows how to handle — but only if
        // it arrives under the code it handles.
        let world = ashfall();
        let ghost = wobu_core::new_id();
        for error in [
            resolved(&world.nodes, ghost, None, &Sliders::neutral(), None).unwrap_err(),
            compiled(&world.nodes, ghost, None, &Sliders::neutral(), None, Budget::unlimited())
                .unwrap_err(),
        ] {
            assert_eq!(serde_json::to_value(&error).unwrap()["code"], "node.not_found");
        }
    }

    #[test]
    fn a_world_with_no_style_guide_and_no_links_resolves_to_a_short_stack() {
        // Every project between `project_create` and the user writing anything
        // is in some version of this state, and the Inspector is on screen for
        // all of it. A thin stack is an answer; an error is not.
        let lonely = Node::new(NodeKind::Prop, "Ash Lantern").unwrap();
        let (id, nodes) = (lonely.id, vec![lonely]);

        let stack = resolved(&nodes, id, None, &Sliders::neutral(), None).unwrap();
        assert_eq!(stack.layers.len(), 1, "the subject and nothing else");
        assert_eq!(stack.layers[0].layer, Layer::Subject);
        assert!(stack.layers[0].fragments.is_empty(), "nothing has been described yet");

        // And it compiles: the framing text is the preset's, so a subject with no
        // description of its own still has a prompt rather than an empty string.
        let compiled =
            compiled(&nodes, id, None, &Sliders::neutral(), None, Budget::unlimited()).unwrap();
        assert_eq!(compiled.preset.id, "prop_orthographic");
        assert!(compiled.prompt.starts_with("orthographic elevation"), "got {}", compiled.prompt);
        assert_eq!(compiled.negative, "");
        assert!(compiled.dropped.is_empty());
    }

    #[test]
    fn an_empty_world_answers_rather_than_panicking() {
        // `World::new` over nothing, which is what a project whose index has not
        // been built yet looks like. It cannot produce a stack, but it must
        // produce the same error as any other missing subject.
        let error =
            resolved(&[], wobu_core::new_id(), None, &Sliders::neutral(), None).unwrap_err();
        assert_eq!(serde_json::to_value(&error).unwrap()["code"], "node.not_found");
    }

    #[test]
    fn a_preset_the_registry_has_never_heard_of_falls_back_to_the_kind_default() {
        // `Generation.preset` is a string that outlives any one build, so a
        // snapshot naming a preset since renamed has to compile to something.
        // Refusing would take the panel down for a record it is trying to show.
        let world = ashfall();
        let sliders = Sliders::neutral();
        let unknown = compiled(
            &world.nodes,
            world.kael,
            Some("silhouette_study"),
            &sliders,
            None,
            Budget::unlimited(),
        )
        .unwrap();
        assert_eq!(unknown.preset.id, "character_sheet");

        // A preset the registry *does* know reweights the same fragments — the
        // costume plate lifts `costume` and all but silences `silhouette`.
        let plate = compiled(
            &world.nodes,
            world.kael,
            Some("costume_plate"),
            &sliders,
            None,
            Budget::unlimited(),
        )
        .unwrap();
        let weight = |c: &CompiledPrompt, section: &str| {
            c.spans.iter().find(|s| s.section == section).map(|s| s.weight)
        };
        assert!(weight(&plate, "costume") > weight(&unknown, "costume"));
        assert!(weight(&plate, "silhouette") < weight(&unknown, "silhouette"));
    }

    #[test]
    fn a_card_turned_down_to_nothing_keeps_its_rows_and_loses_its_words() {
        // The difference between "you turned this off" and "your notes are gone".
        // The panel that exists to explain the prompt must not answer the second
        // when the user did the first, so the fragments stay on the card and the
        // drop report says `silenced` rather than `budget`.
        let world = ashfall();
        let guild = world.nodes.iter().find(|n| n.kind == NodeKind::Culture).unwrap().id;
        let sliders = Sliders::from_pairs([(guild, 0.0)]);

        let stack = resolved(&world.nodes, world.kael, None, &sliders, None).unwrap();
        let card = stack.layers.iter().find(|c| c.node_id == Some(guild)).unwrap();
        assert_eq!(card.slider, 0.0);
        assert_eq!(card.fragments.len(), 1, "the longcoat is still on the card");

        let compiled =
            compiled(&world.nodes, world.kael, None, &sliders, None, Budget::unlimited()).unwrap();
        assert!(!compiled.prompt.contains("longcoat"), "got {}", compiled.prompt);
        let silenced: Vec<&str> = compiled
            .dropped
            .iter()
            .filter(|d| d.reason == DropReason::Silenced)
            .map(|d| d.fragment.section)
            .collect();
        assert_eq!(silenced, ["costume"]);
    }

    #[test]
    fn the_shot_card_is_only_there_once_a_shot_has_been_set_up() {
        // `influence_resolve` is the display path and has no shot until the panel
        // gives it one; `prompt_compile` always has one, because the framing text
        // is part of the prompt a generation would send. Getting this backwards
        // means the box shows a prompt the Generate button would not produce.
        let world = ashfall();
        assert!(!stack(&world).layers.iter().any(|c| c.layer == Layer::Shot));

        let controls = ShotControls {
            label: Some("Turnaround".into()),
            weight: Some(0.5),
            prompt: Some("at dusk in falling ash".into()),
        };
        let framed =
            resolved(&world.nodes, world.kael, None, &Sliders::neutral(), Some(&controls)).unwrap();
        let shot = framed.layers.last().unwrap();
        assert_eq!(shot.layer, Layer::Shot);
        assert_eq!(shot.name, "Turnaround");
        assert_eq!(shot.node_id, None, "the shot is not a node");
        assert_eq!(shot.weight, 0.5);
        assert!(
            shot.fragments.iter().any(|fragment| {
                fragment.section == "user_prompt"
                    && fragment.text.as_deref() == Some("at dusk in falling ash")
            }),
            "the extra shot prompt must be an exact attributed contribution",
        );

        let custom = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            Some(&controls),
            Budget::unlimited(),
        )
        .unwrap();
        assert!(custom.prompt.ends_with("at dusk in falling ash"));

        // And the compiled prompt carries the preset's framing whether or not
        // anyone named the shot.
        assert!(prompt(&world).prompt.ends_with("single subject"));
    }

    #[test]
    fn a_budget_that_bites_reports_what_it_cut_instead_of_truncating() {
        // The acceptance criterion for the whole command: a caller that only
        // received the string could not tell a prompt that fitted from one that
        // had been quietly cut in half.
        let world = ashfall();
        let cramped = compiled(
            &world.nodes,
            world.kael,
            None,
            &Sliders::neutral(),
            None,
            Budget { prompt: Chars::new(40), negative: Chars::new(0) },
        )
        .unwrap();

        assert!(cramped.prompt.chars().count() <= 40, "got {}", cramped.prompt);
        assert_eq!(cramped.negative, "", "the negatives are emptied rather than overrun");
        let cut: Vec<(&str, DropReason)> =
            cramped.dropped.iter().map(|d| (d.fragment.section, d.reason)).collect();
        assert!(cut.contains(&("never", DropReason::Budget)), "got {cut:?}");
        assert!(cut.iter().all(|(_, reason)| *reason == DropReason::Budget), "got {cut:?}");
        // Everything that survived is still attributed, so the panel can say
        // which layer paid for the ones that did not.
        assert!(cramped.spans.iter().all(|s| !s.source_name.is_empty()));
    }

    #[test]
    fn the_sliders_the_panel_sends_arrive_as_the_weights_it_asked_for() {
        // `sliders` crosses as an array of `{ nodeId, value }`. A rename on
        // either side would fail at the bridge, and every drag would go nowhere
        // with nothing on screen to say why.
        let settings: Vec<SliderSetting> =
            serde_json::from_str(r#"[{"nodeId":"01ARZ3NDEKTSV4RRFFQ69G5FAV","value":0.25}]"#)
                .expect("sliders should decode");
        let id: Id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        assert_eq!(sliders_from(Some(settings)).get(id), 0.25);
        // Out of range is clamped rather than refused — the control's range is
        // the engine's business, not the panel's.
        assert_eq!(sliders_from(None).get(id), 1.0, "an untouched card is at full weight");
    }

    #[test]
    fn an_absent_budget_is_unlimited_and_a_partial_one_binds_only_its_own_pool() {
        // No backend has been chosen yet, so there is no real limit to apply and
        // inventing one would drop fragments to fit a number nobody measured.
        assert_eq!(budget_from(None), Budget::unlimited());

        let partial: PromptBudget =
            serde_json::from_str(r#"{"promptChars":900}"#).expect("budget should decode");
        let budget = budget_from(Some(partial));
        assert_eq!(budget.prompt, Chars::new(900));
        assert_eq!(budget.negative, Chars::UNLIMITED, "the two pools are metered separately");
    }
}
