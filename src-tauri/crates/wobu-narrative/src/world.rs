//! Canonical world assertions and character beliefs are independent authored records.
//! Unresolved links and conditions are draft diagnostics, never silently repaired.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::source::{SOURCE_SCHEMA_VERSION, check_version};
use crate::{Condition, EntityId, Name, SceneId, StateSchema, Value};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quest {
    pub id: EntityId,
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub stages: Vec<Name>,
    pub initial: Name,
    #[serde(default)]
    pub transitions: Vec<QuestTransition>,
    #[serde(default)]
    pub scene_ids: Vec<SceneId>,
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
        check_version(yaml)?;
        crate::source::parse_yaml(yaml)
    }

    pub fn to_yaml(&self) -> Result<String> {
        if self.schema_version != SOURCE_SCHEMA_VERSION {
            return Err(Error::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: SOURCE_SCHEMA_VERSION,
            });
        }
        crate::source::print_yaml(self)
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
            let stages: BTreeSet<_> = quest.stages.iter().collect();
            if stages.len() != quest.stages.len() {
                check.issue(quest.id, "stages", "Quest stages must be unique.");
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
