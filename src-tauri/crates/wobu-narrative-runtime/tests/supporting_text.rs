//! Compiling and playing the six supporting text kinds (#167).
//!
//! One fixture project holds a bark, an ambient exchange, a companion reaction,
//! a codex entry, a quest summary and a journal, plus the scene they sit
//! alongside, because the claims worth testing are all about the *relationship*
//! between the two: that a bark is selected without moving the scene cursor,
//! that its wording is gated by the same release rules, and that a save carries
//! both cursors' worth of state.

use std::collections::BTreeMap;

use wobu_narrative::*;
use wobu_narrative_compiler::*;
use wobu_narrative_runtime::{Error, Runtime, Snapshot, TextDelivery, Yield};

#[path = "../../wobu-narrative-compiler/tests/support/reviews.rs"]
mod reviews;

fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}

fn line(speaker: Speaker, body: &str) -> DialogueSlot {
    let mut slot = DialogueSlot::new(speaker);
    slot.variants.push(Variant::new(Text::written(body)));
    slot
}

fn entry(label: &str, lines: Vec<DialogueSlot>) -> TextEntry {
    TextEntry { lines, ..TextEntry::new(label) }
}

fn alarm(raised: bool) -> Condition {
    Condition::Compare(Comparison {
        var: name("alarm_raised"),
        op: CompareOp::Eq,
        value: Operand::Literal(Value::Bool(raised)),
    })
}

fn schema() -> StateSchema {
    StateSchema::new([
        VariableDecl {
            name: name("alarm_raised"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Narrative,
            description: String::new(),
        },
        VariableDecl {
            name: name("relic_found"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Host,
            description: String::new(),
        },
    ])
    .unwrap()
}

/// A minimal scene, so the graph has an entry point and the supporting assets
/// have something to be delivered alongside.
fn scene() -> Scene {
    let mut scene = Scene::new("Cinder Bay");
    let mut beat = Beat::new("Arrival");
    beat.dialogue.push(line(Speaker::Narrator, "Ash falls on the quay."));
    beat.outcomes.push(Outcome::new(Destination::End { label: "arrived".into() }));
    scene.beats.push(beat);
    scene
}

struct Cast {
    guard: EntityId,
    dockhand: EntityId,
    mate: EntityId,
    companion: EntityId,
}

impl Cast {
    /// World entity ids, minted the way the sibling execution fixtures mint
    /// them: this crate does not depend on `wobu-core`, and a narrative id's
    /// raw ULID is the same thing a character id is.
    fn new() -> Cast {
        Cast {
            guard: SceneId::new().raw(),
            dockhand: SceneId::new().raw(),
            mate: SceneId::new().raw(),
            companion: SceneId::new().raw(),
        }
    }

    fn ids(&self) -> Vec<EntityId> {
        vec![self.guard, self.dockhand, self.mate, self.companion]
    }
}

fn cast_of(entity: EntityId, role: &str) -> Participant {
    Participant { entity, role: role.into() }
}

/// A four-line bark under the default shuffled policy.
fn bark(cast: &Cast) -> TextAsset {
    let mut asset = TextAsset::new(TextKind::Bark, "Gate guard", name("player_passes_gate"));
    asset.participants.push(cast_of(cast.guard, "guard"));
    asset.summary = "What the gate guard says to a passer-by.".into();
    for body in ["Move along.", "Papers, if you have them.", "Quiet night.", "Mind the ash."] {
        asset.entries.push(entry(body, vec![line(Speaker::Entity(cast.guard), body)]));
    }
    asset
}

fn ambient(cast: &Cast) -> TextAsset {
    let mut asset = TextAsset::new(TextKind::Ambient, "Short tally", name("market_idle"));
    asset.participants.push(cast_of(cast.dockhand, "dockhand"));
    asset.participants.push(cast_of(cast.mate, "mate"));
    asset.entries.push(entry(
        "Counting",
        vec![
            line(Speaker::Entity(cast.dockhand), "Tally's short again."),
            line(Speaker::Entity(cast.mate), "Then count it twice."),
            line(Speaker::Entity(cast.dockhand), "I counted it three times."),
        ],
    ));
    asset
}

fn reaction(cast: &Cast) -> TextAsset {
    let mut asset = TextAsset::new(TextKind::Reaction, "Relic found", name("relic_taken"));
    asset.participants.push(cast_of(cast.companion, "companion"));
    let mut calm = entry("Calm", vec![line(Speaker::Entity(cast.companion), "Careful with that.")]);
    calm.when = Some(alarm(false));
    let mut alarmed =
        entry("Alarmed", vec![line(Speaker::Entity(cast.companion), "Put it down and run.")]);
    alarmed.when = Some(alarm(true));
    asset.entries.push(calm);
    asset.entries.push(alarmed);
    asset
}

fn codex() -> TextAsset {
    let mut asset = TextAsset::new(TextKind::Codex, "The beacons", name("codex_opened"));
    asset.entries.push(entry(
        "Overview",
        vec![
            line(Speaker::Narrator, "The beacons predate the harbour."),
            line(Speaker::Narrator, "Nobody living remembers who lit the first."),
        ],
    ));
    asset
}

fn quest_summary() -> TextAsset {
    let mut asset = TextAsset::new(TextKind::QuestSummary, "Ashfall", name("quest_log_opened"));
    let mut raised = entry("Alarm raised", vec![line(Speaker::Narrator, "The harbour is awake.")]);
    raised.when = Some(alarm(true));
    asset.entries.push(raised);
    asset
        .entries
        .push(entry("Quiet", vec![line(Speaker::Narrator, "Find out who lit the beacon.")]));
    asset
}

fn journal() -> TextAsset {
    let mut asset = TextAsset::new(TextKind::Journal, "First night", name("day_ends"));
    asset.repeat = RepeatPolicy::Once;
    asset.entries.push(entry("Night one", vec![line(Speaker::Player, "I found the logbook.")]));
    asset.entries.push(entry("Night two", vec![line(Speaker::Player, "The guard knows my name.")]));
    asset
}

fn assets(cast: &Cast) -> Vec<TextAsset> {
    vec![bark(cast), ambient(cast), reaction(cast), codex(), quest_summary(), journal()]
}

fn options(cast: &Cast) -> CompileOptions {
    CompileOptions { known_entities: cast.ids().into_iter().collect(), ..Default::default() }
}

fn compiled(cast: &Cast, texts: &[TextAsset], options: &CompileOptions) -> Graph {
    let report = compile(&[scene()], texts, &schema(), options);
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
    let _ = cast;
    report.graph.unwrap()
}

fn started(graph: Graph, seed: u64) -> Runtime {
    let scene = graph.scenes.keys().next().expect("one scene").clone();
    Runtime::start(
        graph,
        &scene,
        BTreeMap::from([(name("relic_found"), Value::Bool(false))]),
        "playthrough-1".into(),
        seed,
        64,
    )
    .expect("the fixture scene starts")
}

fn deliver(runtime: &mut Runtime, event: &str) -> TextDelivery {
    runtime
        .deliver_text(&name(event))
        .expect("delivery succeeds")
        .expect("the fixture has something eligible")
}

#[test]
fn all_six_kinds_compile_and_deliver() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let graph = compiled(&cast, &texts, &options(&cast));
    assert_eq!(graph.texts.len(), 6);

    let mut runtime = started(graph, 1);
    let expected = [
        ("player_passes_gate", TextKind::Bark),
        ("market_idle", TextKind::Ambient),
        ("relic_taken", TextKind::Reaction),
        ("codex_opened", TextKind::Codex),
        ("quest_log_opened", TextKind::QuestSummary),
        ("day_ends", TextKind::Journal),
    ];
    for (event, kind) in expected {
        let delivery = deliver(&mut runtime, event);
        assert_eq!(delivery.kind, kind, "{event}");
        assert!(!delivery.lines.is_empty(), "{event}");
        assert!(delivery.lines.iter().all(|line| !line.text.is_empty()), "{event}");
        assert!(delivery.lines.iter().all(|line| line.revision.len() == 32), "{event}");
    }
}

#[test]
fn an_unbound_event_is_silence_rather_than_an_error() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);
    assert_eq!(runtime.deliver_text(&name("nothing_listens_here")).unwrap(), None);
}

#[test]
fn the_host_can_discover_the_events_a_package_answers() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let events: Vec<String> =
        started(graph, 1).text_events().iter().map(|e| e.to_string()).collect();
    assert_eq!(
        events,
        [
            "codex_opened",
            "day_ends",
            "market_idle",
            "player_passes_gate",
            "quest_log_opened",
            "relic_taken",
        ]
    );
}

#[test]
fn an_ambient_exchange_delivers_several_speakers_in_order() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);
    let delivery = deliver(&mut runtime, "market_idle");

    assert_eq!(delivery.lines.len(), 3);
    let speakers: Vec<&Speaker> = delivery.lines.iter().map(|line| &line.speaker).collect();
    assert_eq!(speakers[0], &Speaker::Entity(cast.dockhand));
    assert_eq!(speakers[1], &Speaker::Entity(cast.mate));
    assert_eq!(speakers[2], &Speaker::Entity(cast.dockhand));
    // Each line keeps its own identity, which is what makes them separately
    // recordable and translatable rather than one paragraph.
    let mut ids: Vec<&String> = delivery.lines.iter().map(|line| &line.slot).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
}

#[test]
fn standalone_prose_delivers_without_any_choice_being_offered() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);
    let before = runtime.current().unwrap();
    let delivery = deliver(&mut runtime, "codex_opened");

    assert!(delivery.lines.iter().all(|line| line.speaker == Speaker::Narrator));
    // The scene cursor is untouched: reading a codex page is not a story move,
    // and there is no fake choice anywhere in the yield it produced.
    assert_eq!(runtime.current().unwrap(), before);
    assert!(!matches!(before, Yield::Choices { .. }));
}

#[test]
fn conditions_choose_between_entries_of_the_same_asset() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 5);

    let calm = deliver(&mut runtime, "relic_taken");
    assert_eq!(calm.lines[0].text, "Careful with that.");
    assert_eq!(
        deliver(&mut runtime, "quest_log_opened").lines[0].text,
        "Find out who lit the beacon."
    );
}

#[test]
fn a_bark_exhausts_its_bag_before_repeating_and_never_repeats_across_the_join() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let graph = compiled(&cast, &texts, &options(&cast));
    let mut runtime = started(graph, 99);

    let heard: Vec<String> =
        (0..8).map(|_| deliver(&mut runtime, "player_passes_gate").entry).collect();

    let mut first: Vec<&String> = heard[..4].iter().collect();
    first.sort();
    first.dedup();
    assert_eq!(first.len(), 4, "a four-entry bag must contain each entry once: {heard:?}");
    let mut second: Vec<&String> = heard[4..].iter().collect();
    second.sort();
    second.dedup();
    assert_eq!(second.len(), 4, "{heard:?}");
    assert_ne!(heard[3], heard[4], "a new round must not open with the line just heard");
    assert_eq!(
        runtime.text_progress()[&texts[0].id.to_string()].plays,
        8,
        "{:?}",
        runtime.text_progress()
    );
}

#[test]
fn the_seed_decides_the_bark_order_and_the_same_seed_always_agrees() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let options = options(&cast);

    let order = |seed: u64| -> Vec<String> {
        let mut runtime = started(compiled(&cast, &texts, &options), seed);
        (0..4).map(|_| deliver(&mut runtime, "player_passes_gate").entry).collect()
    };
    assert_eq!(order(7), order(7));
    let differs = (0..16).any(|seed| order(seed) != order(7));
    assert!(differs, "some other seed must produce a different bag order");
}

#[test]
fn a_save_taken_mid_bag_resumes_without_repeating_a_line() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let options = options(&cast);
    let graph = compiled(&cast, &texts, &options);

    let mut runtime = started(graph.clone(), 21);
    let mut heard: Vec<String> =
        (0..2).map(|_| deliver(&mut runtime, "player_passes_gate").entry).collect();
    let snapshot = runtime.snapshot();

    // Restore, and finish the round from the save rather than from the runtime
    // that produced it.
    let mut restored = Runtime::restore(graph, snapshot).expect("the save restores");
    heard.extend((0..2).map(|_| deliver(&mut restored, "player_passes_gate").entry));

    let mut seen: Vec<&String> = heard.iter().collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4, "a reload must not re-deal the bag: {heard:?}");
}

#[test]
fn a_save_round_trips_its_repeat_state_through_json() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph.clone(), 3);
    deliver(&mut runtime, "player_passes_gate");
    deliver(&mut runtime, "day_ends");

    let json = serde_json::to_string(&runtime.snapshot()).unwrap();
    let snapshot: Snapshot = serde_json::from_str(&json).unwrap();
    let restored = Runtime::restore(graph, snapshot).expect("the save restores");
    assert_eq!(restored.text_progress(), runtime.text_progress());
}

#[test]
fn a_playthrough_with_no_supporting_text_saves_exactly_the_bytes_it_used_to() {
    let cast = Cast::new();
    let graph = compiled(&cast, &[], &options(&cast));
    let json = serde_json::to_value(started(graph, 1).snapshot()).unwrap();
    // The new field is absent rather than present-and-empty, so an existing save
    // and a save written by this build are the same document.
    assert!(json.get("texts").is_none(), "{json}");
}

#[test]
fn a_project_without_supporting_text_compiles_to_the_graph_it_always_did() {
    let cast = Cast::new();
    let json = serde_json::to_value(compiled(&cast, &[], &options(&cast))).unwrap();
    assert!(json.get("texts").is_none(), "{json}");
}

#[test]
fn a_journal_runs_out_and_stays_out() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);

    assert_eq!(deliver(&mut runtime, "day_ends").lines[0].text, "I found the logbook.");
    assert_eq!(deliver(&mut runtime, "day_ends").lines[0].text, "The guard knows my name.");
    assert_eq!(runtime.deliver_text(&name("day_ends")).unwrap(), None);
}

#[test]
fn a_quest_summary_gives_the_same_answer_to_the_same_state() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);
    let first = deliver(&mut runtime, "quest_log_opened");
    let second = deliver(&mut runtime, "quest_log_opened");
    assert_eq!(first.entry, second.entry);
    assert_eq!(first.lines, second.lines);
    assert_eq!(second.plays, 2, "the read is still counted, it just does not advance");
}

#[test]
fn delivering_supporting_text_cannot_write_narrative_state() {
    let cast = Cast::new();
    let graph = compiled(&cast, &assets(&cast), &options(&cast));
    let mut runtime = started(graph, 1);
    let before = runtime.state().clone();
    for event in ["player_passes_gate", "market_idle", "codex_opened", "day_ends"] {
        let _ = runtime.deliver_text(&name(event)).unwrap();
    }
    assert_eq!(runtime.state(), &before);
}

#[test]
fn a_line_with_no_applicable_wording_fails_and_rolls_the_delivery_back() {
    let cast = Cast::new();
    let mut texts = assets(&cast);
    // Every variant of the reaction's first line is conditioned on state that is
    // false, so the entry is eligible and its wording is not.
    texts[2].entries[0].lines[0].variants[0].when = Some(alarm(true));
    let graph = compiled(&cast, &texts, &options(&cast));
    let mut runtime = started(graph, 1);

    let error = runtime.deliver_text(&name("relic_taken")).unwrap_err();
    assert!(matches!(error, Error::NoMatch(_)), "{error}");
    assert!(runtime.text_progress().is_empty(), "a failed delivery must not be counted");
}

#[test]
fn a_save_naming_an_asset_the_graph_does_not_contain_is_refused() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let graph = compiled(&cast, &texts, &options(&cast));
    let mut runtime = started(graph, 1);
    deliver(&mut runtime, "player_passes_gate");

    // The same playthrough replayed against a graph with the bark removed. The
    // scene, state and hash checks would all pass; only the text history knows.
    let without = compiled(&cast, &texts[1..], &options(&cast));
    let mut snapshot = runtime.snapshot();
    let mut json = serde_json::to_value(&snapshot).unwrap();
    json["graph_hash"] = serde_json::json!(without.hash());
    snapshot = serde_json::from_value(json).unwrap();
    let error = Runtime::restore(without, snapshot).unwrap_err();
    assert!(matches!(error, Error::InvalidState(_)), "{error}");
}

#[test]
fn release_refuses_supporting_text_that_no_reviewer_approved() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let mut options = options(&cast);
    options.profile = Profile::Release;
    options.verified_reviews = reviews::fixture_reviews(&scene());

    let report = compile(&[scene()], &texts, &schema(), &options);
    assert!(report.graph.is_none(), "unapproved supporting text must not reach a release");
    assert!(
        report.diagnostics.iter().any(|d| d.code == "text_not_release_ready" && d.asset.is_some()),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn release_accepts_supporting_text_with_current_approval_evidence() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let scene = scene();
    let mut options = options(&cast);
    options.profile = Profile::Release;
    options.verified_reviews = reviews::fixture_reviews(&scene);
    options.verified_text_reviews = texts.iter().flat_map(reviews::fixture_text_reviews).collect();

    let report = compile(&[scene], &texts, &schema(), &options);
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
}

#[test]
fn an_approval_recorded_against_other_wording_does_not_release_a_bark() {
    let cast = Cast::new();
    let mut texts = assets(&cast);
    let scene = scene();
    let mut options = options(&cast);
    options.profile = Profile::Release;
    options.verified_reviews = reviews::fixture_reviews(&scene);
    options.verified_text_reviews = texts.iter().flat_map(reviews::fixture_text_reviews).collect();

    // The reviewer approved "Move along."; the writer then rewrote it. The slot
    // and variant identities are unchanged, which is precisely the case a
    // revision-keyed approval exists to catch.
    texts[0].entries[0].lines[0].variants[0]
        .text
        .set_body("Move along, quickly.", Provenance::Human);

    let report = compile(&[scene], &texts, &schema(), &options);
    assert!(report.graph.is_none(), "{:?}", report.diagnostics);
}

#[test]
fn a_development_build_still_compiles_unapproved_supporting_text_as_a_warning() {
    let cast = Cast::new();
    let report = compile(&[scene()], &assets(&cast), &schema(), &options(&cast));
    assert!(report.graph.is_some(), "{:?}", report.diagnostics);
    assert!(
        report.diagnostics.iter().all(|d| d.severity == Severity::Warning || d.asset.is_none()),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn broken_supporting_source_blocks_the_graph_at_every_profile() {
    let cast = Cast::new();
    let mut texts = assets(&cast);
    // A codex page voiced by a character: prose attributed to somebody is that
    // person speaking, which is a different asset with different consequences.
    texts[3].entries[0].lines[0].speaker = Speaker::Entity(cast.guard);

    let report = compile(&[scene()], &texts, &schema(), &options(&cast));
    assert!(report.graph.is_none(), "{:?}", report.diagnostics);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_source" && d.severity == Severity::Error),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn a_speaker_who_is_not_in_the_world_blocks_the_graph() {
    let cast = Cast::new();
    let texts = assets(&cast);
    let mut options = options(&cast);
    options.known_entities.remove(&cast.guard);

    let report = compile(&[scene()], &texts, &schema(), &options);
    assert!(report.graph.is_none(), "{:?}", report.diagnostics);
    assert!(
        report.diagnostics.iter().any(|d| d.code == "unknown_entity"),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn an_id_repeated_between_a_scene_and_an_asset_is_reported() {
    let cast = Cast::new();
    let mut scene = scene();
    let mut texts = assets(&cast);
    // The same variant identity in two documents: one locale row would overwrite
    // the other, which is exactly what the uniqueness check exists to prevent.
    let stolen = texts[0].entries[0].lines[0].variants[0].id;
    scene.beats[0].dialogue[0].variants[0].id = stolen;
    texts.truncate(1);

    let report = compile(&[scene], &texts, &schema(), &options(&cast));
    assert!(
        report.diagnostics.iter().any(|d| d.code == "duplicate_id"),
        "{:?}",
        report.diagnostics
    );
    assert!(report.graph.is_none());
}

#[test]
fn supporting_only_runtime_needs_no_scene_and_restores_explicit_repeat_state() {
    let mut asset = TextAsset::new(TextKind::Codex, "Lantern", name("codex_read"));
    asset
        .entries
        .push(entry("Description", vec![line(Speaker::Narrator, "Glass protects the flame.")]));
    let event = asset.trigger.event.clone();
    let graph =
        compile(&[], &[asset], &StateSchema::default(), &CompileOptions::default()).graph.unwrap();
    let mut runner =
        Runtime::start_text(graph.clone(), BTreeMap::new(), "text-run".into(), 17).unwrap();
    assert!(matches!(runner.current().unwrap(), Yield::End { .. }));
    assert_eq!(
        runner.deliver_text(&event).unwrap().unwrap().lines[0].text,
        "Glass protects the flame."
    );
    let snapshot = runner.snapshot();
    let mut restored = Runtime::restore(graph, snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.deliver_text(&event).unwrap(), runner.deliver_text(&event).unwrap());
    assert!(runner.visits().is_empty());
}
