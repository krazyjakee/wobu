//! Deterministic, offline source validation and lowering. No generation or IO.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use wobu_narrative::{
    Condition, Destination, Effect, EntityId, Name, Operand, Owner, Problem, Scene, SceneCatalog,
    Site, Speaker, StateSchema, Value, VarType,
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
    pub profile: Profile,
    /// Complete world membership, not just entities used by this scene.
    pub known_entities: BTreeSet<EntityId>,
    /// Registered argument domains. A variable argument must fit the entire domain.
    pub commands: BTreeMap<Name, Vec<VarType>>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            verified_reviews: BTreeMap::new(),
            profile: Profile::Development,
            known_entities: BTreeSet::new(),
            commands: BTreeMap::new(),
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
    pub scene: String,
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
    pub scene: String,
    pub beat: Option<String>,
    pub slot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledScene {
    pub entry: Option<Condition>,
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

pub fn accepts(ty: &VarType, value: &Value) -> bool {
    match (ty, value) {
        (VarType::Bool, Value::Bool(_)) => true,
        (VarType::Int { min, max }, Value::Int(n)) => min <= n && n <= max,
        (VarType::Enum { members }, Value::Enum(n)) => members.contains(n),
        _ => false,
    }
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

pub fn compile(scenes: &[Scene], schema: &StateSchema, options: &CompileOptions) -> CompileReport {
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
                site,
                severity,
                code: code.into(),
                message,
            })
        };
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
                    dialogue: beat
                        .dialogue
                        .iter()
                        .map(|s| CompiledSlot {
                            id: s.id.to_string(),
                            speaker: s.speaker.clone(),
                            variants: s
                                .variants
                                .iter()
                                .map(|v| CompiledVariant {
                                    id: v.id.to_string(),
                                    when: v.when.clone(),
                                    text: v.text.body.clone(),
                                    revision: v.text.revision.to_string(),
                                })
                                .collect(),
                        })
                        .collect(),
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
        let mut ids =
            vec![(scene_id.clone(), SourceRef { scene: scene_id.clone(), beat: None, slot: None })];
        for beat in &scene.beats {
            let base =
                SourceRef { scene: scene_id.clone(), beat: Some(beat.id.to_string()), slot: None };
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
                first: scene.beats.first().map(|b| b.id.to_string()).unwrap_or_default(),
                beats,
            },
        );
    }
    CompileReport {
        graph: (!diagnostics.iter().any(|d| d.severity == Severity::Error)).then_some(graph),
        diagnostics,
    }
}
