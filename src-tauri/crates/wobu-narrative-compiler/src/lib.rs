//! Deterministic, offline source validation and lowering. No generation or IO.
mod analysis;
pub use analysis::compile_with_analysis;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use wobu_narrative::{
    Condition, Destination, Effect, EntityId, Name, Operand, Owner, Problem, RepeatPolicy, Scene,
    SceneCatalog, Site, Speaker, StateSchema, TextAsset, TextKind, Value, VarType,
};

pub const GRAPH_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Development,
    Release,
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    /// Verified by the host from canonical history and current context. Empty fails closed.
    pub verified_reviews:
        BTreeMap<wobu_narrative::VariantId, wobu_narrative::review::ApprovalEvidence>,
    /// The same evidence for supporting text lines (#167). A separate map rather
    /// than a widened one because the two carry different target shapes, and a
    /// bark whose approval was recorded against a scene target must not verify.
    pub verified_text_reviews:
        BTreeMap<wobu_narrative::VariantId, wobu_narrative::review::TextApprovalEvidence>,
    pub profile: Profile,
    /// Complete world membership, not just entities used by this scene.
    pub known_entities: BTreeSet<EntityId>,
    /// The project's `setting` nodes. A separate set rather than a filter over
    /// `known_entities`, because node kind is not knowable from an id: folding
    /// them together would let a scene be placed in a character.
    pub known_settings: BTreeSet<EntityId>,
    /// Registered argument domains. A variable argument must fit the entire domain.
    pub commands: BTreeMap<Name, Vec<VarType>>,
    /// Repeated wordings somebody has signed off (#209). Empty means every
    /// duplicate is reported, which is the right default: a suppression is an
    /// author's statement and absence of one is not consent.
    pub wording_suppressions: Vec<wobu_narrative::WordingSuppression>,
    /// The project's quests (#207), so their stage objectives reach the graph and
    /// the runtime can report one. Empty is a project with no quests, which
    /// compiles to exactly the graph it did before they were compiled at all.
    pub quests: Vec<wobu_narrative::Quest>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            verified_reviews: BTreeMap::new(),
            verified_text_reviews: BTreeMap::new(),
            profile: Profile::Development,
            known_entities: BTreeSet::new(),
            known_settings: BTreeSet::new(),
            commands: BTreeMap::new(),
            wording_suppressions: Vec::new(),
            quests: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileDiagnostic {
    /// The scene responsible, or empty when `asset` names a supporting text
    /// document instead. Left as a required field rather than made optional so
    /// every existing consumer keeps reading the same key for the same thing.
    pub scene: String,
    /// The supporting text asset responsible (#167), when the diagnostic is not
    /// about a scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// The quest responsible (#207), when the diagnostic is about a quest rather
    /// than a scene or an asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest: Option<String>,
    /// Which stage of that quest. A name rather than an id because a stage has
    /// no id: it is named by the enum value a condition compares against, and
    /// giving it a second identity would create two ways to say which stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Name>,
    pub site: Site,
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileReport {
    pub graph: Option<Graph>,
    pub diagnostics: Vec<CompileDiagnostic>,
}

/// Runtime schema deliberately omits the writer's description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateVariable {
    pub ty: VarType,
    pub default: Value,
    pub owner: Owner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub version: u32,
    pub profile: Profile,
    pub state: BTreeMap<Name, StateVariable>,
    pub commands: BTreeMap<Name, Vec<VarType>>,
    pub scenes: BTreeMap<String, CompiledScene>,
    /// Supporting text assets, keyed by asset id (#167).
    ///
    /// Skipped when empty, so a project that contains no supporting text
    /// compiles to exactly the bytes it did before this field existed — and
    /// therefore to the same graph hash, so existing saves, packages and
    /// scenario tapes keep validating. Adding a bark to a project is a content
    /// change and does move the hash, which is correct: the game now says
    /// something it did not say before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, CompiledText>,
    /// Quests, keyed by quest id (#207).
    ///
    /// Skipped when empty for the reason `texts` is: a project with no quests
    /// compiles to exactly the bytes it did before this field existed, so
    /// existing saves, packages and scenario tapes keep validating against the
    /// same graph hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub quests: BTreeMap<String, CompiledQuest>,
    pub source_map: BTreeMap<String, SourceRef>,
}

impl Graph {
    /// Canonical ordered JSON; no timestamps, source paths or machine identifiers.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("runtime graph contains only JSON-safe types")
    }

    pub fn hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"wobu-narrative/graph/1\0");
        hasher.update(&self.canonical_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    /// Empty when `asset` is set; see [`CompileDiagnostic::scene`].
    pub scene: String,
    pub beat: Option<String>,
    pub slot: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The quest a stage objective belongs to (#207).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledScene {
    pub entry: Option<Condition>,
    /// Where the scene happens, as the `setting` node's id (#206).
    ///
    /// In the graph rather than only in the debug source map, because a release
    /// package carries no source map and a host placing a scene in its world
    /// needs this in a release build — which is the whole point of the field
    /// existing instead of a `Location:` line in the summary. Skipped when
    /// absent, so a project that states no settings compiles to exactly the
    /// bytes it did before and keeps its graph hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting: Option<String>,
    pub first: String,
    pub beats: BTreeMap<String, CompiledBeat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledBeat {
    pub dialogue: Vec<CompiledSlot>,
    pub choices: Vec<CompiledChoice>,
    pub outcomes: Vec<CompiledOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledSlot {
    pub id: String,
    pub speaker: Speaker,
    pub variants: Vec<CompiledVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledVariant {
    pub id: String,
    pub when: Option<Condition>,
    pub text: String,
    pub revision: String,
}

/// One compiled supporting text asset.
///
/// It has no `first`, no targets and no choices, and that absence is the point:
/// the runtime can deliver one of these without touching the scene cursor, so a
/// bark fired during a conversation cannot move the story.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledText {
    pub kind: TextKind,
    /// The host-side event name this asset answers.
    pub event: Name,
    pub when: Option<Condition>,
    pub repeat: RepeatPolicy,
    pub entries: Vec<CompiledTextEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledTextEntry {
    pub id: String,
    pub when: Option<Condition>,
    /// Reuses [`CompiledSlot`] so a bark's wording is the same runtime object as
    /// a scene line's: same variant selection, same string identity, same place
    /// in the exported string table.
    pub lines: Vec<CompiledSlot>,
}

/// One compiled quest: a stage machine and the objective wording for each stage.
///
/// It has no beats, no dialogue and no targets, and that absence is the point —
/// a quest does not move the story, it describes where in the story the player
/// is. The runtime advances it from state and reports the current stage's
/// objective; nothing in here can make a scene happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledQuest {
    pub initial: Name,
    /// Stages in author order. The order is presentation, not priority; the
    /// machine is driven entirely by `transitions`.
    pub stages: Vec<CompiledStage>,
    /// Transitions in author order, which *is* the priority: the first one whose
    /// condition holds wins, exactly as for a beat's outcomes.
    pub transitions: Vec<CompiledTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledStage {
    pub name: Name,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<CompiledObjective>,
}

/// The player-facing wording for a stage, in the shape the string table moves.
///
/// `id`, `text` and `revision` and nothing else, so the package can lift the text
/// out into `strings/en.json` keyed by the same kind of identity a dialogue line
/// uses — which is what makes a locale pack one list rather than two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledObjective {
    pub id: String,
    pub text: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledTransition {
    pub from: Name,
    pub to: Name,
    pub when: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledChoice {
    pub id: String,
    pub label: String,
    pub requires: Option<Condition>,
    pub effects: Vec<Effect>,
    pub to: Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledOutcome {
    pub id: String,
    pub when: Option<Condition>,
    pub effects: Vec<Effect>,
    pub to: Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Beat(String),
    Scene(String),
    End { label: String },
}

impl TryFrom<&Destination> for Target {
    type Error = &'static str;
    fn try_from(value: &Destination) -> Result<Self, Self::Error> {
        Ok(match value {
            Destination::Unresolved {} => {
                return Err("Unresolved destinations cannot be compiled.");
            }
            Destination::Beat(id) => Self::Beat(id.to_string()),
            Destination::Scene(id) => Self::Scene(id.to_string()),
            Destination::End { label } => Self::End { label: label.clone() },
        })
    }
}

pub fn accepts(ty: &wobu_narrative::VarType, value: &Value) -> bool {
    wobu_narrative::evaluate::accepts(ty, value)
}

/// Whether a source argument fits a registered command domain; shared with package validation.
pub fn argument_fits(ty: &VarType, arg: &Operand, schema: &StateSchema) -> bool {
    match arg {
        Operand::Literal(value) => accepts(ty, value),
        Operand::Var(name) => schema.get(name).is_some_and(|decl| match (ty, &decl.ty) {
            (VarType::Bool, VarType::Bool) => true,
            (VarType::Int { min, max }, VarType::Int { min: a, max: b }) => min <= a && b <= max,
            (VarType::Enum { members }, VarType::Enum { members: other }) => {
                other.iter().all(|v| members.contains(v))
            }
            _ => false,
        }),
    }
}

/// Lower validated source into the runtime graph.
///
/// `texts` is a separate list rather than a second kind of scene because the two
/// are structurally different: a scene is a graph the player walks and a
/// supporting text asset is a table the host asks. Passing an empty slice is the
/// explicit way to say a caller has no supporting text, and it compiles to
/// exactly the graph it always did.
pub fn compile(
    scenes: &[Scene],
    texts: &[TextAsset],
    schema: &StateSchema,
    options: &CompileOptions,
) -> CompileReport {
    let mut diagnostics = Vec::new();
    let mut graph = Graph {
        version: GRAPH_VERSION,
        profile: options.profile,
        state: schema
            .iter()
            .map(|d| {
                (
                    d.name.clone(),
                    StateVariable { ty: d.ty.clone(), default: d.default.clone(), owner: d.owner },
                )
            })
            .collect(),
        commands: options.commands.clone(),
        scenes: BTreeMap::new(),
        texts: BTreeMap::new(),
        quests: BTreeMap::new(),
        source_map: BTreeMap::new(),
    };
    let catalog = SceneCatalog::of(scenes.iter().map(|s| s.id));
    let mut ordered: Vec<_> = scenes.iter().collect();
    ordered.sort_by_key(|s| s.id);
    for scene in ordered {
        let scene_id = scene.id.to_string();
        let mut emit = |site, severity, code: &str, message: String| {
            diagnostics.push(CompileDiagnostic {
                scene: scene_id.clone(),
                asset: None,
                quest: None,
                stage: None,
                site,
                severity,
                code: code.into(),
                message,
            })
        };
        if scene.supporting_text.is_some() {
            emit(Site::Scene, Severity::Error, "editorial_projection", "Supporting text must compile from its canonical text asset, not its authoring adapter.".into());
            continue;
        }
        for diagnostic in scene.diagnostics(schema, &catalog) {
            let missing = matches!(diagnostic.problem, Problem::MissingText);
            let severity = if missing && options.profile == Profile::Development {
                Severity::Warning
            } else {
                Severity::Error
            };
            emit(
                diagnostic.site,
                severity,
                if missing { "missing_text" } else { "invalid_source" },
                diagnostic.problem.to_string(),
            );
        }
        for participant in &scene.participants {
            if !options.known_entities.contains(&participant.entity) {
                emit(
                    Site::Participant { entity: participant.entity },
                    Severity::Error,
                    "unknown_entity",
                    format!("world entity {} does not exist", participant.entity),
                );
            }
        }
        for (diagnostic, _) in scene.setting_diagnostics(&options.known_settings) {
            emit(
                diagnostic.site,
                Severity::Error,
                "unknown_setting",
                diagnostic.problem.to_string(),
            );
        }
        let mut beats = BTreeMap::new();
        for beat in &scene.beats {
            for choice in &beat.choices {
                if choice.label.trim().is_empty() {
                    emit(
                        Site::Destination(wobu_narrative::DestinationSite::Choice {
                            beat: beat.id,
                            choice: choice.id,
                        }),
                        if options.profile == Profile::Release {
                            Severity::Error
                        } else {
                            Severity::Warning
                        },
                        "missing_choice_text",
                        "player-facing choice text is empty".into(),
                    );
                }
            }
            for (site, effects) in beat
                .choices
                .iter()
                .map(|c| {
                    (
                        Site::Destination(wobu_narrative::DestinationSite::Choice {
                            beat: beat.id,
                            choice: c.id,
                        }),
                        &c.effects,
                    )
                })
                .chain(beat.outcomes.iter().map(|o| {
                    (
                        Site::Destination(wobu_narrative::DestinationSite::Outcome {
                            beat: beat.id,
                            outcome: o.id,
                        }),
                        &o.effects,
                    )
                }))
            {
                for effect in effects {
                    if let Effect::Command(command) = effect {
                        let valid = options.commands.get(&command.name).is_some_and(|types| {
                            types.len() == command.args.len()
                                && types
                                    .iter()
                                    .zip(&command.args)
                                    .all(|(ty, arg)| argument_fits(ty, arg, schema))
                        });
                        if !valid {
                            emit(
                                site,
                                Severity::Error,
                                "invalid_command",
                                format!(
                                    "command {} is unregistered or has incompatible arguments",
                                    command.name
                                ),
                            );
                        }
                    }
                }
            }
            for slot in &beat.dialogue {
                for variant in &slot.variants {
                    let site = Site::Variant { beat: beat.id, slot: slot.id, variant: variant.id };
                    if variant.text.body.trim().is_empty() {
                        emit(
                            site,
                            if options.profile == Profile::Release {
                                Severity::Error
                            } else {
                                Severity::Warning
                            },
                            "empty_text",
                            "dialogue text is empty".into(),
                        );
                    }
                    if !options.verified_reviews.get(&variant.id).is_some_and(|e| {
                        e.verifies(
                            &wobu_narrative::review::ReviewTarget {
                                scene: scene.id,
                                beat: beat.id,
                                slot: slot.id,
                                variant: Some(variant.id),
                            },
                            &slot.speaker,
                            &variant.text,
                        )
                    }) {
                        emit(
                            site,
                            if options.profile == Profile::Release {
                                Severity::Error
                            } else {
                                Severity::Warning
                            },
                            "text_not_release_ready",
                            "dialogue must be approved and current for release".into(),
                        );
                    }
                }
            }
            beats.insert(
                beat.id.to_string(),
                CompiledBeat {
                    dialogue: beat.dialogue.iter().map(compiled_slot).collect(),
                    choices: beat
                        .choices
                        .iter()
                        .filter_map(|c| {
                            Some(CompiledChoice {
                                id: c.id.to_string(),
                                label: c.label.clone(),
                                requires: c.requires.clone(),
                                effects: c.effects.clone(),
                                to: Target::try_from(&c.to).ok()?,
                            })
                        })
                        .collect(),
                    outcomes: beat
                        .outcomes
                        .iter()
                        .filter_map(|o| {
                            Some(CompiledOutcome {
                                id: o.id.to_string(),
                                when: o.when.clone(),
                                effects: o.effects.clone(),
                                to: Target::try_from(&o.to).ok()?,
                            })
                        })
                        .collect(),
                },
            );
        }
        // IDs are globally unique, including across scenes and element kinds.
        let mut ids = vec![(
            scene_id.clone(),
            SourceRef {
                scene: scene_id.clone(),
                beat: None,
                slot: None,
                asset: None,
                entry: None,
                quest: None,
                stage: None,
            },
        )];
        for beat in &scene.beats {
            let base = SourceRef {
                scene: scene_id.clone(),
                beat: Some(beat.id.to_string()),
                slot: None,
                asset: None,
                entry: None,
                quest: None,
                stage: None,
            };
            ids.push((beat.id.to_string(), base.clone()));
            for choice in &beat.choices {
                ids.push((choice.id.to_string(), base.clone()));
            }
            for outcome in &beat.outcomes {
                ids.push((outcome.id.to_string(), base.clone()));
            }
            for slot in &beat.dialogue {
                let loc = SourceRef { slot: Some(slot.id.to_string()), ..base.clone() };
                ids.push((slot.id.to_string(), loc.clone()));
                for variant in &slot.variants {
                    ids.push((variant.id.to_string(), loc.clone()));
                }
            }
        }
        for (id, loc) in ids {
            if graph.source_map.insert(id.clone(), loc).is_some() {
                emit(
                    Site::Scene,
                    Severity::Error,
                    "duplicate_id",
                    format!("id {id} is repeated in the compilation input"),
                );
            }
        }
        graph.scenes.insert(
            scene_id,
            CompiledScene {
                entry: scene.entry.clone(),
                setting: scene.setting_id.map(|id| id.to_string()),
                first: scene.beats.first().map(|b| b.id.to_string()).unwrap_or_default(),
                beats,
            },
        );
    }

    let mut ordered_texts: Vec<_> = texts.iter().collect();
    ordered_texts.sort_by_key(|asset| asset.id);
    for asset in ordered_texts {
        compile_text(asset, schema, options, &mut graph, &mut diagnostics);
    }

    let mut ordered_quests: Vec<_> = options.quests.iter().collect();
    ordered_quests.sort_by_key(|quest| quest.id);
    for quest in ordered_quests {
        compile_quest(quest, schema, options, &mut graph, &mut diagnostics);
    }

    // The only check here that cannot be answered by reading one document (#209),
    // and therefore the only one that runs after both loops: whether a wording is
    // stored somewhere else as well.
    //
    // A warning at both profiles, deliberately. A repeated line is frequently
    // intended, so refusing a release over one would make the check something to
    // work around rather than to read; and the suppression exists so an intended
    // repetition can be answered once rather than re-explained every build.
    for found in wobu_narrative::duplicated_wording(scenes, texts, &options.wording_suppressions) {
        let first = &found.sites[0];
        let (scene, asset) = match first.container {
            wobu_narrative::WordingContainer::Scene(id) => (id.to_string(), None),
            wobu_narrative::WordingContainer::TextAsset(id) => {
                (String::new(), Some(id.to_string()))
            }
        };
        diagnostics.push(CompileDiagnostic {
            scene,
            asset,
            quest: None,
            stage: None,
            site: first.site,
            severity: Severity::Warning,
            code: "duplicated_wording".into(),
            message: found.message(),
        });
    }

    CompileReport {
        graph: (!diagnostics.iter().any(|d| d.severity == Severity::Error)).then_some(graph),
        diagnostics,
    }
}

/// Lower one quest into its stage machine, and gate its objective wording the way
/// dialogue is gated (#207).
///
/// A stage with no player-facing wording is a task in Development and a blocker
/// in Release, which is the same rule `missing_text` applies to a dialogue slot —
/// and for the same reason. Without it a host has nothing authored to show as
/// "what should I be doing now", so it shows the nearest string it can find; in
/// practice that was the current beat's title, which is a display name written
/// for the writer and frequently names a mistake the player has not made yet.
///
/// Only *reachable* stages are gated. A stage nothing leads to cannot be entered,
/// so requiring wording for it would be busywork with no symptom.
fn compile_quest(
    quest: &wobu_narrative::Quest,
    schema: &StateSchema,
    options: &CompileOptions,
    graph: &mut Graph,
    diagnostics: &mut Vec<CompileDiagnostic>,
) {
    let quest_id = quest.id.to_string();
    let gate =
        if options.profile == Profile::Release { Severity::Error } else { Severity::Warning };
    let mut emit = |stage: Option<&Name>, severity, code: &str, message: String| {
        diagnostics.push(CompileDiagnostic {
            scene: String::new(),
            asset: None,
            quest: Some(quest_id.clone()),
            stage: stage.cloned(),
            // A quest is not inside a scene, so no scene-internal locator
            // describes it. The quest and stage fields above are the selection.
            site: Site::QuestObjective,
            severity,
            code: code.into(),
            message,
        })
    };

    // The structural checks `WorldDocument::diagnose` makes are authoring
    // diagnostics; a compile has to refuse rather than report, because a stage
    // machine whose initial stage is not declared has no runtime meaning at all.
    if !quest.declares(&quest.initial) {
        emit(
            Some(&quest.initial),
            Severity::Error,
            "invalid_quest",
            format!("quest stage `{}` is the initial stage but is not declared", quest.initial),
        );
        return;
    }
    for transition in &quest.transitions {
        for stage in [&transition.from, &transition.to] {
            if !quest.declares(stage) {
                emit(
                    Some(stage),
                    Severity::Error,
                    "invalid_quest",
                    format!("quest transition names stage `{stage}`, which is not declared"),
                );
                return;
            }
        }
        if let Err(error) = schema.check_condition(&transition.when) {
            emit(None, Severity::Error, "invalid_quest", error.to_string());
            return;
        }
    }

    let reachable: BTreeSet<&Name> =
        quest.reachable_stages().into_iter().map(|stage| &stage.name).collect();
    let mut stages = Vec::new();
    for stage in &quest.stages {
        if reachable.contains(&stage.name) && !stage.has_objective() {
            emit(
                Some(&stage.name),
                gate,
                "missing_objective",
                format!(
                    "stage `{}` has no player-facing objective, so a host has nothing authored to                      show as the current objective",
                    stage.name
                ),
            );
        }
        stages.push(CompiledStage {
            name: stage.name.clone(),
            objective: stage.objective.as_ref().map(|objective| CompiledObjective {
                id: objective.id.to_string(),
                text: objective.text.body.clone(),
                revision: objective.text.revision.to_string(),
            }),
        });
    }

    // Objective ids join the one namespace every other wording identity is in, so
    // a string table, a locale row and a recording script are one list.
    for stage in &quest.stages {
        let Some(objective) = &stage.objective else { continue };
        let id = objective.id.to_string();
        let loc = SourceRef {
            scene: String::new(),
            beat: None,
            slot: None,
            asset: None,
            entry: None,
            quest: Some(quest_id.clone()),
            stage: Some(stage.name.clone()),
        };
        if graph.source_map.insert(id.clone(), loc).is_some() {
            emit(
                Some(&stage.name),
                Severity::Error,
                "duplicate_id",
                format!("id {id} is repeated in the compilation input"),
            );
        }
    }

    graph.quests.insert(
        quest_id,
        CompiledQuest {
            initial: quest.initial.clone(),
            stages,
            transitions: quest
                .transitions
                .iter()
                .map(|transition| CompiledTransition {
                    from: transition.from.clone(),
                    to: transition.to.clone(),
                    when: transition.when.clone(),
                })
                .collect(),
        },
    );
}

/// Lower one supporting text asset, and refuse the same things scene dialogue is
/// refused.
///
/// Split out rather than inlined so the two release gates that matter — empty
/// wording and missing approval evidence — are visibly the same rules applied to
/// the same lifecycle, rather than a second, laxer set written for a bark.
fn compile_text(
    asset: &TextAsset,
    schema: &StateSchema,
    options: &CompileOptions,
    graph: &mut Graph,
    diagnostics: &mut Vec<CompileDiagnostic>,
) {
    let asset_id = asset.id.to_string();
    let gate =
        if options.profile == Profile::Release { Severity::Error } else { Severity::Warning };
    let mut emit = |site, severity, code: &str, message: String| {
        diagnostics.push(CompileDiagnostic {
            scene: String::new(),
            asset: Some(asset_id.clone()),
            quest: None,
            stage: None,
            site,
            severity,
            code: code.into(),
            message,
        })
    };

    for diagnostic in asset.diagnostics(schema) {
        // A slot with no wording is a task in Development and a blocker in
        // Release, exactly as it is inside a scene. Everything else a supporting
        // asset can get wrong — an undeclared variable, a speaker outside the
        // cast, a codex page voiced by a character — is invalid source at any
        // profile, because none of it has a defensible runtime meaning.
        let missing = matches!(diagnostic.problem, Problem::MissingText);
        let severity = if missing { gate } else { Severity::Error };
        emit(
            diagnostic.site,
            severity,
            if missing { "missing_text" } else { "invalid_source" },
            diagnostic.problem.to_string(),
        );
    }
    for participant in &asset.participants {
        if !options.known_entities.contains(&participant.entity) {
            emit(
                Site::Participant { entity: participant.entity },
                Severity::Error,
                "unknown_entity",
                format!("world entity {} does not exist", participant.entity),
            );
        }
    }

    let mut entries = Vec::new();
    let mut ids = vec![(
        asset_id.clone(),
        SourceRef {
            scene: String::new(),
            beat: None,
            slot: None,
            asset: Some(asset_id.clone()),
            entry: None,
            quest: None,
            stage: None,
        },
    )];
    for entry in &asset.entries {
        let base = SourceRef {
            scene: String::new(),
            beat: None,
            slot: None,
            asset: Some(asset_id.clone()),
            entry: Some(entry.id.to_string()),
            quest: None,
            stage: None,
        };
        ids.push((entry.id.to_string(), base.clone()));
        for slot in &entry.lines {
            let loc = SourceRef { slot: Some(slot.id.to_string()), ..base.clone() };
            ids.push((slot.id.to_string(), loc.clone()));
            for variant in &slot.variants {
                ids.push((variant.id.to_string(), loc.clone()));
                let site =
                    Site::TextVariant { entry: entry.id, slot: slot.id, variant: variant.id };
                if variant.text.body.trim().is_empty() {
                    emit(site, gate, "empty_text", "supporting text is empty".into());
                }
                let target = wobu_narrative::review::TextTarget {
                    asset: asset.id,
                    entry: entry.id,
                    slot: slot.id,
                    variant: Some(variant.id),
                };
                if !options
                    .verified_text_reviews
                    .get(&variant.id)
                    .is_some_and(|e| e.verifies(&target, &slot.speaker, &variant.text))
                {
                    emit(
                        site,
                        gate,
                        "text_not_release_ready",
                        "supporting text must be approved and current for release".into(),
                    );
                }
            }
        }
        entries.push(CompiledTextEntry {
            id: entry.id.to_string(),
            when: entry.when.clone(),
            lines: entry.lines.iter().map(compiled_slot).collect(),
        });
    }
    for (id, loc) in ids {
        if graph.source_map.insert(id.clone(), loc).is_some() {
            emit(
                Site::TextAsset,
                Severity::Error,
                "duplicate_id",
                format!("id {id} is repeated in the compilation input"),
            );
        }
    }
    graph.texts.insert(
        asset_id,
        CompiledText {
            kind: asset.kind,
            event: asset.trigger.event.clone(),
            when: asset.trigger.when.clone(),
            repeat: asset.repeat,
            entries,
        },
    );
}

/// One dialogue slot lowered to its runtime form.
///
/// Shared by scenes and supporting text so there is exactly one answer to what a
/// compiled line contains, and so a change to that answer cannot reach one and
/// miss the other.
fn compiled_slot(slot: &wobu_narrative::DialogueSlot) -> CompiledSlot {
    CompiledSlot {
        id: slot.id.to_string(),
        speaker: slot.speaker.clone(),
        variants: slot
            .variants
            .iter()
            .map(|v| CompiledVariant {
                id: v.id.to_string(),
                when: v.when.clone(),
                text: v.text.body.clone(),
                revision: v.text.revision.to_string(),
            })
            .collect(),
    }
}
