//! Authoring adapter for the shared slot lifecycle. No synthetic source scene,
//! choice or runtime node is created. Existing receipt targets retain their bytes:
//! container/section IDs carry the original asset/entry identity in this adapter.
use crate::{Beat, BeatId, Scene, SceneId, TextAsset};

impl TextAsset {
    pub fn editorial_scene(&self) -> Scene {
        let mut scene = Scene::new(&self.name);
        scene.id = SceneId::from_raw(self.id.raw());
        scene.editorial_head = self.editorial_head;
        scene.summary = self.summary.clone();
        scene.participants = self.participants.clone();
        scene.entry = self.trigger.when.clone();
        scene.tombstones = self.tombstones.clone();
        scene.beats = self
            .entries
            .iter()
            .map(|entry| {
                let mut beat = Beat::new(&entry.label);
                beat.id = BeatId::from_raw(entry.id.raw());
                beat.dialogue = entry.lines.clone();
                beat.must_convey = self.must_convey.clone();
                beat.must_not_reveal = self.must_not_reveal.clone();
                beat
            })
            .collect();
        let mut metadata = self.clone();
        metadata.editorial_head = None;
        for entry in &mut metadata.entries {
            entry.lines.clear();
        }
        scene.supporting_text = Some(Box::new(metadata));
        scene
    }
}

impl Scene {
    /// Restore only the shared editorial fields; authoring structure lives in
    /// the captured asset metadata, not in invented scene edges.
    pub fn editorial_text(&self) -> Option<TextAsset> {
        let mut asset = self.supporting_text.as_deref()?.clone();
        if self.id.raw() != asset.id.raw()
            || self.beats.len() != asset.entries.len()
            || self.name != asset.name
            || self.summary != asset.summary
            || self.participants != asset.participants
            || self.entry != asset.trigger.when
            || self.tombstones != asset.tombstones
            || self.act_id.is_some()
            || self.arc_id.is_some()
            || !self.tag_ids.is_empty()
            || asset.editorial_head.is_some()
        {
            return None;
        }
        asset.editorial_head = self.editorial_head;
        for entry in &mut asset.entries {
            let beat = self.beats.iter().find(|beat| beat.id.raw() == entry.id.raw())?;
            if !beat.choices.is_empty()
                || !beat.outcomes.is_empty()
                || !beat.intents.is_empty()
                || beat.title != entry.label
                || beat.must_convey != asset.must_convey
                || beat.must_not_reveal != asset.must_not_reveal
                || !entry.lines.is_empty()
            {
                return None;
            }
            entry.lines = beat.dialogue.clone();
        }
        Some(asset)
    }
}
