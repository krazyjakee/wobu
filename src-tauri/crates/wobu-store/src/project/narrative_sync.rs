//! Mutable narrative replication uses our own acknowledged base and editorial
//! checks. Peer claims never become overwrite preconditions.
use super::Project;
use crate::{
    Error, NarrativeFileKind, Result, SourceSave,
    atomic::{self},
    narrative::{records::NarrativeRecordKind, registry},
};
use serde::{Deserialize, Serialize};
use wobu_narrative::{GenerationPolicy, ReviewState, SceneDocument};

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
            if kind == NarrativeFileKind::Scene && !preserves_editorial(old, &incoming.text)? {
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
fn preserves_editorial(old: &str, new: &str) -> Result<bool> {
    let parse = |text| {
        SceneDocument::parse(text).map_err(|e| Error::Malformed {
            path: "narrative/scenes".into(),
            reason: e.to_string(),
        })
    };
    let old = parse(old)?.scene;
    let new = parse(new)?.scene;
    if old.id != new.id {
        return Ok(false);
    }
    for (_, slot) in old.dialogue_slots() {
        for variant in &slot.variants {
            let text = &variant.text;
            if text.lifecycle.policy == GenerationPolicy::Generated {
                continue;
            }
            let next = new
                .dialogue_slots()
                .find(|(_, s)| s.id == slot.id)
                .and_then(|(_, s)| s.variants.iter().find(|v| v.id == variant.id));
            let Some(next) = next else { return Ok(false) };
            if next.text.body != text.body
                || next.text.provenance != text.provenance
                || next.text.lifecycle.policy != text.lifecycle.policy
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
