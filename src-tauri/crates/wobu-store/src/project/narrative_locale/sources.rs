use super::*;
use wobu_narrative::{Freshness, GenerationPolicy as PolicyKind, Scene, Speaker, VariantId};
impl Project {
    pub fn locale_sources(&self) -> Result<BTreeMap<String, SourceLine>> {
        Ok(self.locale_capture()?.0)
    }
    pub(super) fn locale_capture(
        &self,
    ) -> Result<(BTreeMap<String, SourceLine>, Vec<super::super::narrative_review::ReviewSnapshot>)>
    {
        let fingerprint = self.narrative_fingerprint()?;
        let scenes = self.scene_catalog()?;
        let texts = self.text_catalog()?;
        if !scenes.unreadable.is_empty() || !texts.unreadable.is_empty() {
            return Err(invalid("Repair unreadable narrative sources before localisation."));
        }
        let mut ids: Vec<_> = scenes
            .scenes
            .iter()
            .map(|s| s.id)
            .chain(texts.assets.iter().map(|t| wobu_narrative::SceneId::from_raw(t.id.raw())))
            .collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(invalid("Scene and text identities collide."));
        }
        let mut result = BTreeMap::new();
        let snapshots = self.review_snapshots(&ids, None)?;
        for snapshot in &snapshots {
            let id = snapshot.scene().id;
            let view = snapshot.view_with_proposals(Vec::new())?;
            let scene = snapshot.scene();
            for line in &view.lines {
                let (Some(variant), Some(text)) = (line.target.variant, &line.text) else {
                    continue;
                };
                let beat = scene
                    .beats
                    .iter()
                    .find(|b| b.id == line.target.beat)
                    .ok_or_else(|| invalid("Missing source section."))?;
                let unlocks: Vec<_> = snapshot
                    .history
                    .iter()
                    .filter(|event| {
                        locked(&event.before, variant) && !locked(&event.after, variant)
                    })
                    .map(|event| event.id)
                    .collect();
                let source = SourceLine {
                    id: variant.to_string(),
                    slot: line.target.slot.to_string(),
                    container: id.to_string(),
                    speaker: match line.speaker {
                        Speaker::Narrator => "Narrator".into(),
                        Speaker::Player => "Player".into(),
                        Speaker::Entity(entity) => snapshot.characters[entity.to_string()]["name"]
                            .as_str()
                            .unwrap_or("Missing speaker")
                            .to_string(),
                    },
                    text: text.body.clone(),
                    revision: text.revision.to_string(),
                    guard: hash(&(
                        text.revision.clone(),
                        &line.context_revision,
                        &unlocks,
                        locked(scene, variant),
                    )),
                    context: format!("{} / {}\n{}", scene.name, beat.title, scene.summary),
                    delivery_notes: serde_json::to_string(&(
                        (&beat.must_convey, &beat.must_not_reveal),
                        scene.supporting_text.as_ref().map(|a| (&a.kind, &a.trigger, &a.repeat)),
                    ))?,
                    placeholders: locale::format::placeholders(&text.body),
                    ready: line.approval_valid
                        && line.freshness == Freshness::Current
                        && locked(scene, variant),
                };
                if result.insert(source.id.clone(), source).is_some() {
                    return Err(invalid("Duplicate localisation stable ID."));
                }
            }
            // Choice labels depend on the containing dialogue's protection. Retain
            // unlock decisions so relocking cannot revive a prior translation approval.
            let choice_unlocks: Vec<_> = if scene.beats.iter().all(|beat| beat.choices.is_empty()) {
                Vec::new()
            } else {
                snapshot
                    .history
                    .iter()
                    .filter(|event| has_unlock(&event.before, &event.after))
                    .map(|event| event.id)
                    .collect()
            };
            // Choice labels are structural authored text, without a separate review policy.
            // Their containing dialogue must be approved and locked before export.
            for beat in &scene.beats {
                for choice in &beat.choices {
                    let source=SourceLine {id:choice.id.to_string(),slot:choice.id.to_string(),container:id.to_string(),speaker:"Player".into(),text:choice.label.clone(),revision:hash(&choice.label),guard:hash(&(choice,(&beat.must_convey,&beat.must_not_reveal),&choice_unlocks)),context:format!("{} / {} / choice",scene.name,beat.title),delivery_notes:"Structural choice label; source guard includes its condition and effects.".into(),placeholders:locale::format::placeholders(&choice.label),ready:view.lines.iter().all(|l|l.approval_valid&&l.freshness==Freshness::Current&&l.target.variant.is_some_and(|v|locked(scene,v)))};
                    if result.insert(source.id.clone(), source).is_some() {
                        return Err(invalid("Duplicate choice localisation ID."));
                    }
                }
            }
            snapshot.check_observations(self)?;
        }
        if fingerprint != self.narrative_fingerprint()? {
            return Err(invalid("Narrative changed while capturing localisation source."));
        }
        Ok((result, snapshots))
    }
}
fn locked(scene: &Scene, id: VariantId) -> bool {
    locked_variants(scene).any(|variant| variant == id)
}
fn locked_variants(scene: &Scene) -> impl Iterator<Item = VariantId> + '_ {
    let asset_lock = scene.supporting_text.as_ref().is_some_and(|a| a.policy == PolicyKind::Locked);
    scene.dialogue_slots().flat_map(move |(_, slot)| {
        slot.variants
            .iter()
            .filter(move |v| {
                asset_lock
                    || slot.policy == PolicyKind::Locked
                    || v.text.lifecycle.policy == PolicyKind::Locked
            })
            .map(|v| v.id)
    })
}
fn has_unlock(before: &Scene, after: &Scene) -> bool {
    // Collect each historical scene once instead of rescanning all its variants
    // for every ID. Hash-set iteration order never enters the persisted guard.
    let before: std::collections::HashSet<_> = locked_variants(before).collect();
    let after: std::collections::HashSet<_> = locked_variants(after).collect();
    !before.is_subset(&after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_narrative::{DialogueSlot, Name, Text, TextAsset, TextEntry, TextKind, Variant};

    #[test]
    fn effective_lock_sets_preserve_asset_slot_and_variant_protection() {
        let mut asset = TextAsset::new(TextKind::Codex, "Harbor", Name::new("read").unwrap());
        let mut entry = TextEntry::new("Arrival");
        let mut slot = DialogueSlot::new(Speaker::Narrator);
        slot.variants.push(Variant::new(Text::written("The harbor is quiet.")));
        slot.variants.push(Variant::new(Text::written("The harbor is busy.")));
        let first = slot.variants[0].id;
        let second = slot.variants[1].id;
        entry.lines.push(slot);
        asset.entries.push(entry);
        let mut before = asset.editorial_scene();
        before.beats[0].dialogue[0].variants[0].text.lifecycle.policy = PolicyKind::Locked;
        let mut after = before.clone();
        after.beats[0].dialogue[0].variants[0].text.lifecycle.policy = PolicyKind::Edited;
        after.beats[0].dialogue[0].variants[1].text.lifecycle.policy = PolicyKind::Locked;
        // Equal lock counts are not equal protected identities.
        assert!(has_unlock(&before, &after));
        assert!(locked(&before, first));
        assert!(!locked(&before, second));
        before.beats[0].dialogue[0].policy = PolicyKind::Locked;
        after.beats[0].dialogue[0].policy = PolicyKind::Locked;
        assert!(!has_unlock(&before, &after));
        assert!(locked(&after, first));
        assert!(locked(&after, second));
        before.supporting_text.as_mut().unwrap().policy = PolicyKind::Locked;
        after.supporting_text.as_mut().unwrap().policy = PolicyKind::Locked;
        after.beats[0].dialogue[0].policy = PolicyKind::Edited;
        assert!(!has_unlock(&before, &after));
        after.supporting_text.as_mut().unwrap().policy = PolicyKind::Edited;
        assert!(has_unlock(&before, &after));
    }
}
