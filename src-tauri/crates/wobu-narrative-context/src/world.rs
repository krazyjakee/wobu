use crate::resolve::Builder;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use wobu_narrative::{Belief, EntityId, KnowledgeProvenance};

impl Builder<'_> {
    pub fn world(&mut self, speaker: Option<EntityId>, participants: &BTreeSet<EntityId>) {
        let world = self.input.world;
        let mut seen = BTreeSet::new();
        for (id, _) in world.records() {
            if !seen.insert(id) {
                self.diagnostic(
                    "duplicate_identity",
                    &format!("world/{id}"),
                    "World records share an identity. Repair duplicates before generation.",
                    true,
                );
            }
        }
        let restrictions: Vec<_> = world
            .restrictions
            .iter()
            .filter(|r| {
                r.characters.is_empty() || speaker.is_some_and(|id| r.characters.contains(&id))
            })
            .map(|r| (r.id.to_string(), r))
            .collect();
        self.query("restrictions", json!({"speaker":speaker,"all_characters":true}), &restrictions);
        let mut forbidden = BTreeSet::new();
        for (_, restriction) in restrictions {
            let source = format!("world/restrictions/{}", restriction.id);
            // Malformed release conditions fail closed.
            if self.active(&restriction.until, &source) == Some(true) {
                continue;
            }
            forbidden.insert(restriction.fact);
            self.dependency(&source, restriction);
            let fact = world.facts.iter().find(|f| f.id == restriction.fact);
            self.dependency(&format!("world/facts/{}", restriction.fact), &fact);
            if fact.is_none() {
                self.diagnostic(
                    "missing_source",
                    &source,
                    "Forbidden fact source is missing; repair the restriction.",
                    true,
                );
            }
            self.fragment(
                "future_restriction",
                &source,
                true,
                json!({"restriction":restriction,"forbidden_fact":fact}),
            );
        }
        let claims: Vec<_> = world
            .knowledge
            .iter()
            .filter(|k| Some(k.character) == speaker)
            .map(|k| (k.id.to_string(), k))
            .collect();
        self.query("speaker_knowledge", json!({"character":speaker}), &claims);
        let mut beliefs: BTreeMap<EntityId, Vec<Belief>> = BTreeMap::new();
        let mut available = BTreeSet::new();
        for (_, claim) in claims {
            let source = format!("world/knowledge/{}", claim.id);
            if self.active(&claim.when, &source) != Some(true) {
                continue;
            }
            self.dependency(&source, claim);
            let fact = world.facts.iter().find(|f| f.id == claim.fact);
            self.dependency(&format!("world/facts/{}", claim.fact), &fact);
            if fact.is_none() {
                self.diagnostic(
                    "missing_source",
                    &source,
                    "Knowledge refers to a deleted or missing canonical fact.",
                    true,
                );
                continue;
            }
            if fact.is_some_and(|fact| fact.assertion.trim().is_empty()) {
                self.diagnostic(
                    "missing_source",
                    &source,
                    "Canonical fact has an empty assertion.",
                    true,
                );
            }
            match &claim.provenance {
                KnowledgeProvenance::Told { by } => {
                    let origin = self.input.characters.get(by);
                    self.dependency(
                        &format!("character/{by}/identity"),
                        &origin.map(|c| (&c.id, &c.name)),
                    );
                    if origin.is_none() {
                        self.diagnostic(
                            "missing_source",
                            &source,
                            "The character named as the source of this knowledge is missing.",
                            true,
                        );
                    }
                }
                KnowledgeProvenance::Rumour { source: origin } if origin.trim().is_empty() => self
                    .diagnostic(
                        "missing_source",
                        &source,
                        "Rumour provenance has no authored source.",
                        true,
                    ),
                KnowledgeProvenance::Inferred { reason } if reason.trim().is_empty() => self
                    .diagnostic(
                        "missing_source",
                        &source,
                        "Inferred knowledge has no authored reasoning.",
                        true,
                    ),
                _ => {}
            }
            let prior = beliefs.entry(claim.fact).or_default();
            if prior.iter().any(|belief| belief != &claim.belief) {
                self.diagnostic("conflicting_beliefs",&source,"Simultaneously active claims disagree about this fact. All claims remain visible; resolve the conflict before generation.",true);
            }
            prior.push(claim.belief.clone());
            if forbidden.contains(&claim.fact) {
                self.diagnostic("forbidden_knowledge",&source,"This claim is active but the fact is under a future-revelation restriction. It is excluded from usable knowledge.",false);
                continue;
            }
            if claim.belief == Belief::True {
                available.insert(claim.fact);
            }
            self.fragment(
                "knowledge",
                &source,
                false,
                json!({"claim":claim,"canonical_fact":fact}),
            );
        }
        let relations: Vec<_> = world
            .relationships
            .iter()
            .filter(|r| Some(r.from) == speaker && participants.contains(&r.to))
            .map(|r| (r.id.to_string(), r))
            .collect();
        self.query("directed_relationships", json!({"from":speaker,"to":participants}), &relations);
        let mut relationship_values = BTreeMap::new();
        for (_, relation) in relations {
            let source = format!("world/relationships/{}", relation.id);
            if self.active(&relation.when, &source) == Some(true) {
                let key = (relation.from, relation.to, relation.kind.clone());
                if relationship_values
                    .insert(key, &relation.value)
                    .is_some_and(|prior| prior != &relation.value)
                {
                    self.diagnostic("conflicting_relationships",&source,"Active directed relationships of the same kind disagree. Resolve the competing values before generation.",true);
                }
                self.dependency(&source, relation);
                self.fragment("relationship", &source, false, relation);
            }
        }
        // Events aren't public knowledge. All referenced facts must be known true;
        // no inferred recency ordering or automatic acquisition from event attendance.
        let events: Vec<_> = world
            .events
            .iter()
            .filter(|e| {
                speaker.is_some_and(|id| e.entity_ids.contains(&id))
                    || e.fact_ids.iter().any(|id| available.contains(id))
            })
            .map(|e| (e.id.to_string(), e))
            .collect();
        self.query("relevant_events", json!({"speaker":speaker,"known_facts":available}), &events);
        for (_, event) in events {
            let source = format!("world/events/{}", event.id);
            if self.active(&event.when, &source) != Some(true) {
                continue;
            }
            if event.fact_ids.iter().any(|id| !world.facts.iter().any(|fact| fact.id == *id)) {
                self.diagnostic(
                    "missing_source",
                    &source,
                    "Relevant event refers to a deleted canonical fact.",
                    true,
                );
            }
            if event.fact_ids.is_empty() || !event.fact_ids.iter().all(|id| available.contains(id))
            {
                self.diagnostic("unavailable_event",&source,"Event summary excluded: each referenced fact must be known true and unrestricted for this speaker.",false);
                continue;
            }
            self.dependency(&source, event);
            self.fragment("event", &source, false, event);
        }
    }
}
