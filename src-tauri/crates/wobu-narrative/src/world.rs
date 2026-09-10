//! Canonical world assertions and character beliefs are independent authored records.
//! Unresolved links and conditions are draft diagnostics, never silently repaired.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::id::VariantId;
use crate::scene::Text;
use crate::source::{SOURCE_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, check_version_for};
use crate::{Condition, EntityId, Name, SceneId, StateSchema, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedClassification {
    pub id: EntityId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub id: EntityId,
    pub name: String,
    pub assertion: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub entity_ids: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Belief {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum KnowledgeProvenance {
    Witnessed,
    Told { by: EntityId },
    Rumour { source: String },
    Inferred { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeClaim {
    pub id: EntityId,
    pub name: String,
    pub character: EntityId,
    pub fact: EntityId,
    pub belief: Belief,
    pub provenance: KnowledgeProvenance,
    #[serde(default = "always")]
    pub when: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub id: EntityId,
    pub name: String,
    pub from: EntityId,
    pub to: EntityId,
    pub kind: Name,
    pub value: Value,
    #[serde(default = "always")]
    pub when: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldEvent {
    pub id: EntityId,
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub fact_ids: Vec<EntityId>,
    #[serde(default)]
    pub entity_ids: Vec<EntityId>,
    #[serde(default = "always")]
    pub when: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestTransition {
    pub from: Name,
    pub to: Name,
    #[serde(default = "always")]
    pub when: Condition,
}

/// One stage of a quest, and what the player is told to do while in it.
///
/// A stage used to be a bare [`Name`] — `available`, `completed` — and nothing in
/// the model carried a player-facing sentence for "what should I be doing now".
/// A host with nothing authored to show falls back to the nearest string it can
/// find, and in practice that was the current beat's title: a display name
/// written for the writer, shown for a beat the player has not reached, which
/// puts an objective on screen that names a mistake before it happens.
///
/// The bare form is still accepted and still written back unchanged, so an
/// existing World file round-trips byte for byte and a stage nobody has written
/// an objective for costs nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestStage {
    pub name: Name,
    /// `None` is a stage with no objective authored yet. A task in Development
    /// and a blocker in Release, exactly as a dialogue slot with no wording is.
    pub objective: Option<QuestObjective>,
}

impl QuestStage {
    /// A stage with nothing authored for the player yet.
    pub fn new(name: Name) -> QuestStage {
        QuestStage { name, objective: None }
    }

    /// Whether there is wording a host could show. Empty wording is the same
    /// answer as no wording: neither is something to put in a quest log.
    pub fn has_objective(&self) -> bool {
        self.objective.as_ref().is_some_and(|o| !o.text.body.trim().is_empty())
    }
}

impl From<Name> for QuestStage {
    fn from(name: Name) -> QuestStage {
        QuestStage::new(name)
    }
}

/// The player-facing wording for one stage.
///
/// A [`VariantId`] and a [`Text`], which is to say the same pair every other
/// piece of authored wording in this model is: the id is what a locale row, a
/// recording and an approval are filed under, and the [`Revision`](crate::Revision)
/// inside the text is what they are keyed to. Giving an objective its own shape
/// would have meant a seventh kind of wording that the string table, the review
/// state and the freshness rules all had to learn about separately.
///
/// There is no `when`. A stage *is* the condition — the quest's transitions say
/// when the player is in it — so conditional objective wording would be two
/// statements of the same thing that can disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestObjective {
    pub id: VariantId,
    pub text: Text,
}

impl QuestObjective {
    /// Wording somebody typed.
    pub fn written(body: impl Into<String>) -> QuestObjective {
        QuestObjective { id: VariantId::new(), text: Text::written(body) }
    }
}

/// The bare name and the structured form are one type, and the bare one is
/// written back bare.
///
/// Not `#[serde(untagged)]` on a two-variant enum, because then a stage would be
/// two types everywhere in the codebase and every reader would start with a
/// match. One struct with a hand-written pair of impls keeps `stage.name` the
/// answer to "which stage is this" regardless of how the file spelled it.
impl Serialize for QuestStage {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match &self.objective {
            // Byte-identical to what was read. An existing World file that nobody
            // has added an objective to must not be rewritten by being opened:
            // it is a file a collaborator merges.
            None => self.name.serialize(serializer),
            Some(objective) => {
                let mut out = serializer.serialize_struct("QuestStage", 2)?;
                out.serialize_field("name", &self.name)?;
                out.serialize_field("objective", objective)?;
                out.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for QuestStage {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Structured {
            name: Name,
            #[serde(default)]
            objective: Option<QuestObjective>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Bare(Name),
            Structured(Structured),
        }
        Ok(match Either::deserialize(deserializer)? {
            Either::Bare(name) => QuestStage { name, objective: None },
            Either::Structured(Structured { name, objective }) => QuestStage { name, objective },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quest {
    pub id: EntityId,
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub stages: Vec<QuestStage>,
    pub initial: Name,
    #[serde(default)]
    pub transitions: Vec<QuestTransition>,
    #[serde(default)]
    pub scene_ids: Vec<SceneId>,
}

impl Quest {
    /// The stage names, in author order.
    ///
    /// The order is the author's and is preserved, because a declared enum of
    /// quest stages is compared against this list and a set would make that
    /// comparison depend on hash order.
    pub fn stage_names(&self) -> Vec<Name> {
        self.stages.iter().map(|stage| stage.name.clone()).collect()
    }

    /// Whether this quest declares a stage by that name.
    pub fn declares(&self, stage: &Name) -> bool {
        self.stages.iter().any(|one| &one.name == stage)
    }

    pub fn stage(&self, name: &Name) -> Option<&QuestStage> {
        self.stages.iter().find(|one| &one.name == name)
    }

    /// The stages a playthrough can actually be in: the initial stage, and
    /// everything a transition can lead to from one that is already reachable.
    ///
    /// Reachability here is over the *authored graph* and deliberately ignores
    /// conditions: deciding whether a condition can ever hold needs a search over
    /// state, and answering "unreachable" wrongly in the reassuring direction
    /// would excuse a missing objective the player will see. A stage nothing
    /// leads to is excluded, because requiring wording for a stage that cannot
    /// be entered would be busywork with no symptom.
    pub fn reachable_stages(&self) -> Vec<&QuestStage> {
        let mut reached: BTreeSet<&Name> = BTreeSet::new();
        if self.declares(&self.initial) {
            reached.insert(&self.initial);
        }
        // At most one new stage per pass, so this terminates on a cyclic quest.
        for _ in 0..self.stages.len() {
            let grown: Vec<&Name> = self
                .transitions
                .iter()
                .filter(|t| reached.contains(&t.from) && self.declares(&t.to))
                .map(|t| &t.to)
                .collect();
            let before = reached.len();
            reached.extend(grown);
            if reached.len() == before {
                break;
            }
        }
        self.stages.iter().filter(|stage| reached.contains(&stage.name)).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FutureRestriction {
    pub id: EntityId,
    pub name: String,
    pub fact: EntityId,
    /// Empty means all characters; otherwise only the listed characters.
    #[serde(default)]
    pub characters: Vec<EntityId>,
    /// Required explicitly: `never` keeps a restriction in force indefinitely.
    pub until: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDocument {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acts: Vec<NamedClassification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arcs: Vec<NamedClassification>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<NamedClassification>,
    #[serde(default)]
    pub facts: Vec<Fact>,
    #[serde(default)]
    pub knowledge: Vec<KnowledgeClaim>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub events: Vec<WorldEvent>,
    #[serde(default)]
    pub quests: Vec<Quest>,
    #[serde(default)]
    pub restrictions: Vec<FutureRestriction>,
}

impl Default for WorldDocument {
    fn default() -> Self {
        Self {
            schema_version: SOURCE_SCHEMA_VERSION,
            acts: vec![],
            arcs: vec![],
            tags: vec![],
            facts: vec![],
            knowledge: vec![],
            relationships: vec![],
            events: vec![],
            quests: vec![],
            restrictions: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDiagnostic {
    pub record_id: Option<EntityId>,
    pub field: String,
    pub message: String,
}

impl WorldDocument {
    pub fn parse(yaml: &str) -> Result<Self> {
        let version = check_version_for(yaml, WORLD_SCHEMA_VERSION)?;
        if version == 1 {
            let value: serde_json::Value = crate::source::parse_yaml(yaml)?;
            if ["acts", "arcs", "tags"].iter().any(|key| value.get(key).is_some()) {
                return Err(crate::source::require_v2());
            }
            // A bare stage name is version-1 shape and stays readable; a stage
            // carrying an objective is not, and a version-1 file claiming one is
            // refused rather than read and then written back as something else.
            if value["quests"]
                .as_array()
                .is_some_and(|quests| quests.iter().any(structured_stages))
            {
                return Err(crate::source::require_v2());
            }
        }
        crate::source::parse_yaml(yaml)
    }

    pub fn to_yaml(&self) -> Result<String> {
        if !(1..=WORLD_SCHEMA_VERSION).contains(&self.schema_version) {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: WORLD_SCHEMA_VERSION,
            });
        }
        if self.schema_version == 1
            && (!self.acts.is_empty()
                || !self.arcs.is_empty()
                || !self.tags.is_empty()
                || self
                    .quests
                    .iter()
                    .any(|quest| quest.stages.iter().any(|stage| stage.objective.is_some())))
        {
            return Err(crate::source::require_v2());
        }
        crate::source::print_yaml(self)
    }

    /// Only explicit guarded writes upgrade a parsed or absent version-1 world.
    pub fn for_save(&self) -> Result<Self> {
        if !(1..=WORLD_SCHEMA_VERSION).contains(&self.schema_version) {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: WORLD_SCHEMA_VERSION,
            });
        }
        let mut document = self.clone();
        document.schema_version = WORLD_SCHEMA_VERSION;
        Ok(document)
    }

    /// Names are presentation. Identity and references survive changing the name.
    pub fn records(&self) -> impl Iterator<Item = (EntityId, &str)> {
        self.facts
            .iter()
            .map(|r| (r.id, r.name.as_str()))
            .chain(self.knowledge.iter().map(|r| (r.id, r.name.as_str())))
            .chain(self.relationships.iter().map(|r| (r.id, r.name.as_str())))
            .chain(self.events.iter().map(|r| (r.id, r.name.as_str())))
            .chain(self.quests.iter().map(|r| (r.id, r.name.as_str())))
            .chain(self.restrictions.iter().map(|r| (r.id, r.name.as_str())))
            .chain(
                self.acts
                    .iter()
                    .chain(&self.arcs)
                    .chain(&self.tags)
                    .map(|r| (r.id, r.name.as_str())),
            )
    }

    pub fn diagnose(
        &self,
        schema: &StateSchema,
        characters: &BTreeSet<EntityId>,
        entities: &BTreeSet<EntityId>,
        scenes: &BTreeSet<SceneId>,
    ) -> Vec<WorldDiagnostic> {
        let mut check = WorldCheck {
            issues: vec![],
            schema,
            characters,
            facts: self.facts.iter().map(|fact| fact.id).collect(),
        };
        let mut seen = BTreeSet::new();
        for (id, name) in self.records() {
            if !seen.insert(id) {
                check.issue(id, "id", "This identity is used by more than one world record.");
            }
            if name.trim().is_empty() {
                check.issue(id, "name", "Give this record a name.");
            }
        }
        for fact in &self.facts {
            for entity in &fact.entity_ids {
                if !entities.contains(entity) {
                    check.issue(fact.id, "entity_ids", &format!("Entity {entity} does not exist."));
                }
            }
            if fact.assertion.trim().is_empty() {
                check.issue(fact.id, "assertion", "Write the canonical assertion for this fact.");
            }
        }
        for claim in &self.knowledge {
            check.character(claim.id, "character", claim.character);
            check.fact(claim.id, "fact", claim.fact);
            if let KnowledgeProvenance::Told { by } = claim.provenance {
                check.character(claim.id, "provenance.by", by);
            }
            check.condition(claim.id, "when", &claim.when);
        }
        for relationship in &self.relationships {
            check.character(relationship.id, "from", relationship.from);
            check.character(relationship.id, "to", relationship.to);
            check.condition(relationship.id, "when", &relationship.when);
        }
        for event in &self.events {
            for entity in &event.entity_ids {
                if !entities.contains(entity) {
                    check.issue(
                        event.id,
                        "entity_ids",
                        &format!("Entity {entity} does not exist."),
                    );
                }
            }
            for fact in &event.fact_ids {
                check.fact(event.id, "fact_ids", *fact);
            }
            check.condition(event.id, "when", &event.when);
        }
        for quest in &self.quests {
            let stages: BTreeSet<_> = quest.stages.iter().map(|stage| &stage.name).collect();
            if stages.len() != quest.stages.len() {
                check.issue(quest.id, "stages", "Quest stages must be unique.");
            }
            // A reachable stage with nothing to show is a task here and a Release
            // blocker in the compiler — the same pair of answers a dialogue slot
            // with no wording gets. Reported on the Quest form, because that is
            // where the person who would write it is looking.
            for stage in quest.reachable_stages() {
                // Only an absent one. Wording that is present but blank is its own
                // diagnostic below, and saying both would be two rows for one
                // mistake. The compiler's Release gate treats them alike, which is
                // right there: neither is shippable.
                if stage.objective.is_none() {
                    check.issue(
                        quest.id,
                        "stages.objective",
                        &format!(
                            "Stage `{}` has no player-facing objective. Without one a host has \
                             nothing authored to show as the current objective.",
                            stage.name
                        ),
                    );
                }
            }
            for stage in &quest.stages {
                let Some(objective) = &stage.objective else { continue };
                // An objective is authored wording, so the two things that can be
                // wrong with any authored wording are wrong with this too. Both
                // are reported rather than repaired: resealing a revision is what
                // would break the locale row keyed to it.
                if objective.text.body.trim().is_empty() {
                    check.issue(
                        quest.id,
                        "stages.objective",
                        &format!(
                            "Stage `{}` has an empty objective. Write what the player should do, \
                             or remove it.",
                            stage.name
                        ),
                    );
                }
                if !objective.text.revision_matches() {
                    check.issue(
                        quest.id,
                        "stages.objective",
                        &format!(
                            "Stage `{}`'s recorded revision does not describe its objective text. \
                             Anything keyed to it — a translation, an approval — has stopped \
                             matching.",
                            stage.name
                        ),
                    );
                }
            }
            if !stages.contains(&quest.initial) {
                check.issue(
                    quest.id,
                    "initial",
                    "The initial stage is not declared in this quest.",
                );
            }
            for transition in &quest.transitions {
                for (field, stage) in
                    [("transitions.from", &transition.from), ("transitions.to", &transition.to)]
                {
                    if !stages.contains(stage) {
                        check.issue(
                            quest.id,
                            field,
                            &format!("Quest stage `{stage}` is not declared."),
                        );
                    }
                }
                check.condition(quest.id, "transitions.when", &transition.when);
            }
            for scene in &quest.scene_ids {
                if !scenes.contains(scene) {
                    check.issue(quest.id, "scene_ids", &format!("Scene {scene} does not exist."));
                }
            }
        }
        for restriction in &self.restrictions {
            check.fact(restriction.id, "fact", restriction.fact);
            for character in &restriction.characters {
                check.character(restriction.id, "characters", *character);
            }
            check.condition(restriction.id, "until", &restriction.until);
        }
        check.issues
    }
}

struct WorldCheck<'a> {
    issues: Vec<WorldDiagnostic>,
    schema: &'a StateSchema,
    characters: &'a BTreeSet<EntityId>,
    facts: BTreeSet<EntityId>,
}

impl WorldCheck<'_> {
    fn issue(&mut self, id: EntityId, field: &str, message: &str) {
        self.issues.push(WorldDiagnostic {
            record_id: Some(id),
            field: field.into(),
            message: message.into(),
        });
    }
    fn character(&mut self, id: EntityId, field: &str, target: EntityId) {
        if !self.characters.contains(&target) {
            self.issue(id, field, &format!("Character {target} does not exist."));
        }
    }
    fn fact(&mut self, id: EntityId, field: &str, target: EntityId) {
        if !self.facts.contains(&target) {
            self.issue(id, field, &format!("Fact {target} does not exist."));
        }
    }
    fn condition(&mut self, id: EntityId, field: &str, condition: &Condition) {
        if let Err(error) = self.schema.check_condition(condition) {
            self.issue(id, field, &error.to_string());
        }
    }
}

fn always() -> Condition {
    Condition::Always
}

/// Whether any of this quest's stages is written as a map rather than a name.
fn structured_stages(quest: &serde_json::Value) -> bool {
    quest["stages"].as_array().is_some_and(|stages| stages.iter().any(|stage| stage.is_object()))
}
