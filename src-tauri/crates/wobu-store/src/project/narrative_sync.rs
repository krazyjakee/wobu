//! Mutable narrative replication uses our own acknowledged base and editorial
//! checks. Peer claims never become overwrite preconditions.
use super::Project;
use crate::{
    Error, NarrativeFileKind, Result, SourceSave,
    atomic::{self},
    narrative::{records::NarrativeRecordKind, registry},
};
use serde::{Deserialize, Serialize};
use wobu_narrative::{
    DialogueSlot, GenerationPolicy, ReviewState, SceneDocument, TextAssetDocument,
};

pub const MAX_NARRATIVE_FILE_BYTES: usize = 2 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeSyncEntry {
    pub rel: String,
    pub hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeIncoming {
    pub rel: String,
    pub hash: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NarrativeApplied {
    Agreed { changed: bool },
    Conflict { path: String },
    Deleted,
}
impl Project {
    pub fn narrative_manifest(&self) -> Result<Vec<NarrativeSyncEntry>> {
        let mut result = Vec::new();
        for entry in registry::observe(self.root())? {
            if let Some(reason) = entry.error {
                return Err(Error::Malformed { path: entry.rel.into(), reason });
            }
            result.push(NarrativeSyncEntry { rel: entry.rel, hash: entry.hash });
        }
        result.sort_by(|a, b| sync_order(&a.rel).cmp(&sync_order(&b.rel)).then(a.rel.cmp(&b.rel)));
        Ok(result)
    }
    pub fn narrative_outgoing(
        &self,
        entry: &NarrativeSyncEntry,
    ) -> Result<Option<NarrativeIncoming>> {
        validate_entry(entry)?;
        let Some((text, stamp)) = registry::read(self.root(), &entry.rel)? else { return Ok(None) };
        if stamp.hash != entry.hash {
            return Ok(None);
        }
        validate_bytes(&entry.rel, &entry.hash, &text)?;
        let (_, _, doc) = registry::parse(&entry.rel, &text)?;
        registry::validate_references(self.root(), registry::classify(&entry.rel).unwrap(), &doc)?;
        Ok(Some(NarrativeIncoming { rel: entry.rel.clone(), hash: entry.hash.clone(), text }))
    }
    /// A known old peer base is behind our local edit, not a competing edit.
    /// The opposite direction will offer our newer bytes during this exchange.
    pub fn narrative_wants(&self, peer: &str, entry: &NarrativeSyncEntry) -> Result<bool> {
        if self.narrative_outgoing(entry)?.is_some() {
            self.record_narrative_agreed(peer, entry)?;
            return Ok(false);
        }
        let current = registry::read(self.root(), &entry.rel)?;
        Ok(!(current.is_some()
            && self.index.narrative_base(peer, &entry.rel)?.as_ref() == Some(&entry.hash)))
    }
    pub fn record_narrative_agreed(&self, peer: &str, entry: &NarrativeSyncEntry) -> Result<()> {
        validate_entry(entry)?;
        self.index.record_narrative_base(peer, &entry.rel, &entry.hash)
    }
    pub fn apply_narrative_from_peer(
        &mut self,
        peer: &str,
        incoming: &NarrativeIncoming,
    ) -> Result<NarrativeApplied> {
        self.ensure_writable()?;
        validate_bytes(&incoming.rel, &incoming.hash, &incoming.text)?;
        let kind = registry::classify(&incoming.rel)
            .ok_or_else(|| Error::NoSuchNode(incoming.rel.clone()))?;
        if self.active_narrative_deletion(&incoming.rel, &incoming.hash)? {
            return Ok(NarrativeApplied::Deleted);
        }
        let current = registry::read(self.root(), &incoming.rel)?;
        if current.as_ref().is_some_and(|(_, stamp)| stamp.hash == incoming.hash) {
            // Matching bytes still need valid portable bindings/dependencies.
            let (_, _, doc) = registry::parse(&incoming.rel, &incoming.text)?;
            registry::validate_references(self.root(), kind, &doc)?;
            self.record_narrative_agreed(
                peer,
                &NarrativeSyncEntry { rel: incoming.rel.clone(), hash: incoming.hash.clone() },
            )?;
            return Ok(NarrativeApplied::Agreed { changed: false });
        }
        let immutable = matches!(
            kind,
            NarrativeFileKind::Object
                | NarrativeFileKind::ReceiptBinding
                | NarrativeFileKind::Tombstone
                | NarrativeFileKind::Restoration
                | NarrativeFileKind::Record(NarrativeRecordKind::Receipt)
        );
        let base = self.index.narrative_base(peer, &incoming.rel)?;
        let mut can_apply = current.is_none()
            || current.as_ref().is_some_and(|(_, stamp)| base.as_ref() == Some(&stamp.hash));
        if let Some((old, _)) = &current {
            registry::parse(&incoming.rel, old)?;
            if immutable || kind == NarrativeFileKind::Record(NarrativeRecordKind::Policy) {
                can_apply = false;
            }
            let preserved = match kind {
                NarrativeFileKind::Scene => preserves_editorial(
                    &scene_slots(old, &incoming.rel)?,
                    &scene_slots(&incoming.text, &incoming.rel)?,
                ),
                NarrativeFileKind::Text => preserves_editorial(
                    &text_slots(old, &incoming.rel)?,
                    &text_slots(&incoming.text, &incoming.rel)?,
                ),
                _ => true,
            };
            if !preserved {
                can_apply = false;
            }
        }
        if kind == NarrativeFileKind::Text {
            // The same refusal scene dialogue gets below: an approval attests to
            // a wording, so a peer offering approved text whose stored revision
            // no longer describes it is offering an approval nobody gave.
            let next = TextAssetDocument::parse(&incoming.text)
                .map_err(|e| Error::Malformed {
                    path: incoming.rel.clone().into(),
                    reason: e.to_string(),
                })?
                .asset;
            if next.lines().flat_map(|(_, slot)| &slot.variants).any(|variant| {
                variant.text.lifecycle.review == ReviewState::Approved
                    && !variant.text.revision_matches()
            }) {
                can_apply = false;
            }
            if self
                .text_catalog()?
                .assets
                .iter()
                .any(|entry| entry.id == next.id && entry.rel != incoming.rel)
            {
                can_apply = false;
            }
        }
        if kind == NarrativeFileKind::Scene {
            let next = SceneDocument::parse(&incoming.text)
                .map_err(|e| Error::Malformed {
                    path: incoming.rel.clone().into(),
                    reason: e.to_string(),
                })?
                .scene;
            if next.dialogue_slots().flat_map(|(_, slot)| &slot.variants).any(|variant| {
                variant.text.lifecycle.review == ReviewState::Approved
                    && !variant.text.revision_matches()
            }) {
                can_apply = false;
            }
            if self
                .scene_catalog()?
                .scenes
                .iter()
                .any(|entry| entry.id == next.id && entry.rel != incoming.rel)
            {
                can_apply = false;
            }
        }
        if !can_apply {
            return self.park_narrative_peer(peer, incoming);
        }
        let (_, _, doc) = registry::parse(&incoming.rel, &incoming.text)?;
        registry::validate_references(self.root(), kind, &doc)?;
        let outcome = if immutable {
            match self.write_narrative_immutable(&incoming.rel, &incoming.text) {
                Ok(stamp) => SourceSave::Saved(stamp),
                Err(Error::AlreadyExists(_)) => return self.park_narrative_peer(peer, incoming),
                Err(e) => return Err(e),
            }
        } else {
            // The precondition comes from our fresh read, after our own base
            // comparison. No expected hash supplied by a peer is trusted.
            self.write_narrative_text(
                &incoming.rel,
                &incoming.text,
                current.as_ref().map(|(_, stamp)| stamp),
            )?
        };
        match outcome {
            SourceSave::Saved(_) => {
                self.record_narrative_agreed(
                    peer,
                    &NarrativeSyncEntry { rel: incoming.rel.clone(), hash: incoming.hash.clone() },
                )?;
                Ok(NarrativeApplied::Agreed { changed: true })
            }
            SourceSave::Conflict { conflict_path } => {
                Ok(NarrativeApplied::Conflict { path: conflict_path })
            }
        }
    }
    fn park_narrative_peer(
        &self,
        peer: &str,
        incoming: &NarrativeIncoming,
    ) -> Result<NarrativeApplied> {
        let target = registry::safe_path(self.root(), &incoming.rel)?;
        // A repeated unresolved conflict should not manufacture another copy
        // every polling round. Existing losing bytes remain the recovery owner.
        for (rel, path) in crate::narrative::conflict_paths(self.root()) {
            if path.parent() == target.parent()
                && path
                    .file_name()
                    .and_then(|v| v.to_str())
                    .and_then(crate::conflict::parse)
                    .is_some_and(|name| {
                        target
                            .file_name()
                            .is_some_and(|file| file == name.target_file_name().as_str())
                    })
                && atomic::read_stamped(&path)?
                    .is_some_and(|(_, stamp)| stamp.hash == incoming.hash)
            {
                return Ok(NarrativeApplied::Conflict { path: rel });
            }
        }
        let (path, _) = atomic::park_conflict(self.root(), &target, &incoming.text, peer)?;
        Ok(NarrativeApplied::Conflict {
            path: crate::paths::to_rel_string(path.strip_prefix(self.root()).unwrap_or(&path)),
        })
    }
}
pub fn validate_entry(entry: &NarrativeSyncEntry) -> Result<()> {
    if entry.rel.len() > 512
        || registry::classify(&entry.rel).is_none()
        || !registry::valid_hash(&entry.hash)
    {
        return Err(Error::Malformed {
            path: entry.rel.clone().into(),
            reason: "Invalid narrative manifest identity or path.".into(),
        });
    }
    Ok(())
}
fn validate_bytes(rel: &str, hash: &str, text: &str) -> Result<()> {
    validate_entry(&NarrativeSyncEntry { rel: rel.into(), hash: hash.into() })?;
    if text.len() > MAX_NARRATIVE_FILE_BYTES || atomic::hash_bytes(text.as_bytes()) != hash {
        return Err(Error::Malformed {
            path: rel.into(),
            reason: "Narrative transfer exceeds its size bound or differs from its content hash."
                .into(),
        });
    }
    registry::parse(rel, text)?;
    Ok(())
}
fn sync_order(rel: &str) -> u8 {
    match registry::classify(rel) {
        Some(NarrativeFileKind::ReceiptBinding) => 0,
        Some(NarrativeFileKind::Object) => 1,
        Some(NarrativeFileKind::Tombstone) => 2,
        Some(NarrativeFileKind::Restoration) => 3,
        Some(NarrativeFileKind::Publication) => 5,
        _ => 4,
    }
}
/// One document's identity and its dialogue slots, which is everything the
/// editorial protection below reads.
///
/// Reducing both document kinds to this shape is what lets a bark and a scene
/// share the rule verbatim. Writing the loop twice would work today and would
/// be the obvious place for the two to drift the first time somebody tightened
/// one of them.
type Slots = (String, Vec<DialogueSlot>);

fn scene_slots(text: &str, rel: &str) -> Result<Slots> {
    let scene = SceneDocument::parse(text)
        .map_err(|e| Error::Malformed { path: rel.into(), reason: e.to_string() })?
        .scene;
    let slots = scene.dialogue_slots().map(|(_, slot)| slot.clone()).collect();
    Ok((scene.id.to_string(), slots))
}

fn text_slots(text: &str, rel: &str) -> Result<Slots> {
    let asset = TextAssetDocument::parse(text)
        .map_err(|e| Error::Malformed { path: rel.into(), reason: e.to_string() })?
        .asset;
    let slots = asset.lines().map(|(_, slot)| slot.clone()).collect();
    Ok((asset.id.to_string(), slots))
}

/// Whether an incoming document leaves every hand-written and locked wording
/// exactly as it was.
///
/// A peer may not replace, weaken or remove text a person wrote, however new it
/// claims its revision to be. Generated wording nobody has touched is the one
/// exception, because that is precisely the text a newer draft is allowed to
/// supersede.
fn preserves_editorial((old_id, old_slots): &Slots, (new_id, new_slots): &Slots) -> bool {
    if old_id != new_id {
        return false;
    }
    for slot in old_slots {
        for variant in &slot.variants {
            let text = &variant.text;
            if text.lifecycle.policy == GenerationPolicy::Generated {
                continue;
            }
            let next = new_slots
                .iter()
                .find(|s| s.id == slot.id)
                .and_then(|s| s.variants.iter().find(|v| v.id == variant.id));
            let Some(next) = next else { return false };
            if next.text.body != text.body
                || next.text.provenance != text.provenance
                || next.text.lifecycle.policy != text.lifecycle.policy
            {
                return false;
            }
        }
    }
    true
}
