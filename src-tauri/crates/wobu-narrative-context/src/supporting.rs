//! Supporting intent and explicitly linked sources use the same attributed
//! resolver. A source link does not grant a character knowledge of its contents.
use crate::resolve::Builder;
use serde_json::json;
use wobu_narrative::{Belief, EntityId, SourceLink};

impl Builder<'_> {
    pub fn supporting(&mut self, speaker: Option<EntityId>) {
        let Some(asset) = self.input.scene.supporting_text.as_deref() else { return };
        let entry = asset
            .entries
            .iter()
            .find(|entry| entry.id.raw() == self.result.options.selection.beat.raw());
        let source = format!("text/{}", asset.id);
        self.fragment("supporting_text", &source, true, json!({
            "kind": asset.kind, "delivery": asset.kind.delivery(), "trigger": asset.trigger,
            "selection_policy": asset.repeat, "entry": entry.map(|entry| json!({"id":entry.id,"when":entry.when})),
            "instruction": match asset.kind {
                wobu_narrative::TextKind::Bark => "Write a brief standalone bark for this host trigger.",
                wobu_narrative::TextKind::Ambient => "Write only the selected speaker's line in this ordered overheard exchange.",
                wobu_narrative::TextKind::Reaction => "Write the companion's reaction to the authored trigger.",
                wobu_narrative::TextKind::Codex => "Write reference prose, without invented player choices.",
                wobu_narrative::TextKind::QuestSummary => "Write a concise quest-log summary for the supplied state.",
                wobu_narrative::TextKind::Journal => "Write the supplied voice's journal passage for the supplied state."
            }
        }));
        if asset.kind == wobu_narrative::TextKind::Ambient {
            let lines = self.input.scene.beats.iter().find(|beat| beat.id == self.result.options.selection.beat).map(|beat| beat.dialogue.iter().map(|slot| {
                let wording = slot.variants.iter().find(|variant| self.active(variant.when.as_ref().unwrap_or(&wobu_narrative::Condition::Always), &source) == Some(true));
                json!({"slot":slot.id,"speaker":slot.speaker,"wording":if slot.id == self.result.options.selection.slot {None} else {wording.map(|variant| &variant.text.body)}})
            }).collect::<Vec<_>>()).unwrap_or_default();
            self.fragment("ordered_exchange", &source, true, lines);
        }
        if let Some(condition) = entry.and_then(|entry| entry.when.as_ref())
            && self.active(condition, &source) != Some(true)
        {
            self.diagnostic(
                "inactive_text_entry",
                &source,
                "This entry is unavailable in the supplied scenario.",
                true,
            );
        }
        for link in &asset.sources {
            match link {
                SourceLink::Fact(id) => {
                    let fact = self.input.world.facts.iter().find(|fact| fact.id == *id);
                    let address = format!("world/facts/{id}");
                    self.dependency(&address, &fact);
                    if fact.is_none() {
                        self.missing_link(&address);
                    } else if self.link_fact_available(*id, speaker) {
                        self.fragment("linked_fact", &address, true, fact);
                    } else {
                        self.diagnostic("unavailable_source", &address, "Linked fact is not usable knowledge in this scenario; its assertion is excluded.", false);
                    }
                }
                SourceLink::Event(id) => {
                    let event = self.input.world.events.iter().find(|event| event.id == *id);
                    let address = format!("world/events/{id}");
                    self.dependency(&address, &event);
                    if let Some(event) = event {
                        let active = self.active(&event.when, &address) == Some(true);
                        let known = !event.fact_ids.is_empty()
                            && event
                                .fact_ids
                                .iter()
                                .all(|id| self.link_fact_available(*id, speaker));
                        if active && known {
                            self.fragment("linked_event", &address, true, event);
                        } else {
                            self.diagnostic("unavailable_source", &address, "Linked event is inactive or its facts are unavailable; its summary is excluded.", false);
                        }
                    } else {
                        self.missing_link(&address);
                    }
                }
                SourceLink::Quest(id) => {
                    let quest = self.input.world.quests.iter().find(|quest| quest.id == *id);
                    let address = format!("world/quests/{id}");
                    self.dependency(&address, &quest);
                    if let Some(quest) = quest {
                        // A whole quest synopsis can describe future stages. It is
                        // editorial intent, never a licence to reveal the ending.
                        self.fragment("linked_quest", &address, true, json!({"id":quest.id,"name":quest.name,"summary":quest.summary,"instruction":"Authored quest intent, not evidence that future stages have happened. Use the supplied state and forbidden revelations."}));
                    } else {
                        self.missing_link(&address);
                    }
                }
                SourceLink::Character(id) => {
                    let character = self.input.characters.get(id);
                    let address = format!("character/{id}/narrative_voice");
                    self.dependency(&address, &character);
                    if character.is_some() {
                        self.fragment("linked_character", &address, true, character);
                    } else {
                        self.missing_link(&address);
                    }
                }
                SourceLink::Scene(id) => {
                    let scene = self.linked_scenes.get(id);
                    let address = format!("scene/{id}");
                    self.dependency(&address, &scene);
                    if let Some(scene) = scene {
                        self.fragment("linked_scene", &address, true, json!({"id":id,"name":scene.name,"summary":scene.summary,"instruction":"Authored scene intent only; no claim that this scene has occurred. Respect supplied state and forbidden revelations."}));
                    } else {
                        self.missing_link(&address);
                    }
                }
            }
        }
    }
    fn missing_link(&mut self, source: &str) {
        self.diagnostic(
            "missing_source",
            source,
            "A linked supporting-text source no longer exists.",
            true,
        );
    }
    fn link_fact_available(&mut self, id: EntityId, speaker: Option<EntityId>) -> bool {
        if !self.input.world.facts.iter().any(|fact| fact.id == id) {
            return false;
        }
        for restriction in &self.input.world.restrictions {
            if restriction.fact == id
                && (restriction.characters.is_empty()
                    || speaker.is_some_and(|speaker| restriction.characters.contains(&speaker)))
                && self
                    .active(&restriction.until, &format!("world/restrictions/{}", restriction.id))
                    != Some(true)
            {
                return false;
            }
        }
        let Some(speaker) = speaker else { return true };
        self.input.world.knowledge.iter().any(|claim| {
            claim.character == speaker
                && claim.fact == id
                && claim.belief == Belief::True
                && self.active(&claim.when, &format!("world/knowledge/{}", claim.id)) == Some(true)
        })
    }
}
