//! Explicit portable deletions and restores. Absence never means peer deletion.
use super::Project;
use crate::{
    Error, NarrativeFileKind, Result, SourceSave,
    atomic::{self, Stamp},
    narrative::{
        publication::{NarrativeDeletion, NarrativeRestoration},
        records::RECORD_VERSION,
        registry,
    },
};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeDeletionView {
    pub deletion: NarrativeDeletion,
    pub restored: bool,
}
impl Project {
    pub fn narrative_deletions(&self) -> Result<Vec<NarrativeDeletionView>> {
        let (deletions, restorations) = self.deletion_records()?;
        Ok(deletions
            .into_iter()
            .map(|deletion| {
                let restored = restorations.iter().any(|restore| restore.deletion == deletion.id);
                NarrativeDeletionView { deletion, restored }
            })
            .collect())
    }
    fn deletion_records(&self) -> Result<(Vec<NarrativeDeletion>, Vec<NarrativeRestoration>)> {
        let mut deletions = Vec::new();
        let mut restorations = Vec::new();
        for (rel, _) in registry::paths(self.root())? {
            let kind = registry::classify(&rel);
            if !matches!(kind, Some(NarrativeFileKind::Tombstone | NarrativeFileKind::Restoration))
            {
                continue;
            }
            let Some((text, _)) = registry::read(self.root(), &rel)? else { continue };
            let (_, _, value) = registry::parse(&rel, &text)?;
            if kind == Some(NarrativeFileKind::Tombstone) {
                deletions.push(serde_json::from_value(value)?)
            } else {
                restorations.push(serde_json::from_value(value)?)
            }
        }
        Ok((deletions, restorations))
    }
    /// Returns false for a stale precondition; no delete is inferred from a
    /// different version or a missing source file.
    pub fn delete_narrative_file(&mut self, rel: &str, expected: &Stamp) -> Result<bool> {
        self.ensure_writable()?;
        let Some((original, stamp)) = registry::read(self.root(), rel)? else { return Ok(false) };
        if stamp.hash != expected.hash {
            return Ok(false);
        }
        let (_, name, document) = registry::parse(rel, &original)?;
        if registry::classify(rel)
            == Some(NarrativeFileKind::Record(crate::NarrativeRecordKind::Receipt))
        {
            self.bind_receipt(&serde_json::from_value(document)?)?;
        }
        let deletion = NarrativeDeletion {
            schema_version: RECORD_VERSION,
            id: wobu_core::new_id(),
            name,
            target: rel.into(),
            hash: stamp.hash,
            original,
        };
        deletion.validate()?;
        self.write_narrative_immutable(
            &deletion.rel(),
            &(serde_json::to_string_pretty(&deletion)? + "\n"),
        )?;
        let removed = self.apply_deletion(&deletion)?;
        self.reconcile_narrative()?;
        Ok(removed)
    }
    pub fn restore_narrative_deletion(&mut self, id: Id) -> Result<SourceSave> {
        self.ensure_writable()?;
        let views = self.narrative_deletions()?;
        let view = views
            .iter()
            .find(|view| view.deletion.id == id)
            .ok_or_else(|| Error::NoSuchNode(id.to_string()))?;
        let deletion = &view.deletion;
        // A restore is an explicit operation. Merely copying old source bytes
        // into the folder or receiving an old peer version does not revoke it.
        if !view.restored {
            let restore = NarrativeRestoration {
                schema_version: RECORD_VERSION,
                id: wobu_core::new_id(),
                deletion: id,
            };
            self.write_narrative_immutable(
                &restore.rel(),
                &(serde_json::to_string_pretty(&restore)? + "\n"),
            )?;
        }
        let current = registry::read(self.root(), &deletion.target)?;
        if let Some((_, stamp)) = &current
            && stamp.hash == deletion.hash
        {
            return Ok(SourceSave::Saved(stamp.clone()));
        }
        // Never overwrite a newer surviving edit during recovery.
        let outcome = self.write_narrative_text(&deletion.target, &deletion.original, None)?;
        self.reconcile_narrative()?;
        Ok(outcome)
    }
    /// Called after a complete sync and on recovery. Restore markers can be
    /// delivered before their tombstone, but cannot restore unknown content.
    pub fn apply_narrative_deletions(&mut self) -> Result<bool> {
        self.ensure_writable()?;
        let (deletions, restorations) = self.deletion_records()?;
        let revoked: std::collections::BTreeSet<_> =
            restorations.iter().map(|r| r.deletion).collect();
        let active: Vec<_> = deletions.iter().filter(|d| !revoked.contains(&d.id)).collect();
        let mut changed = false;
        for deletion in &active {
            changed |= self.apply_deletion(deletion)?;
        }
        for restoration in restorations {
            let Some(deletion) = deletions.iter().find(|d| d.id == restoration.deletion) else {
                return Err(Error::Malformed {
                    path: restoration.rel().into(),
                    reason: "Restoration references a missing deletion record.".into(),
                });
            };
            if active.iter().any(|active| active.target == deletion.target) {
                continue;
            }
            if registry::read(self.root(), &deletion.target)?.is_none() {
                changed |= matches!(
                    self.write_narrative_text(&deletion.target, &deletion.original, None)?,
                    SourceSave::Saved(_)
                );
            }
        }
        if changed {
            self.reconcile_narrative()?;
        }
        Ok(changed)
    }
    pub fn active_narrative_deletion(&self, rel: &str, hash: &str) -> Result<bool> {
        Ok(self.narrative_deletions()?.iter().any(|view| {
            !view.restored && view.deletion.target == rel && view.deletion.hash == hash
        }))
    }
    fn apply_deletion(&mut self, deletion: &NarrativeDeletion) -> Result<bool> {
        deletion.validate()?;
        let target = registry::safe_path(self.root(), &deletion.target)?;
        let Some((_, stamp)) = registry::read(self.root(), &deletion.target)? else {
            return Ok(false);
        };
        if stamp.hash != deletion.hash {
            return Ok(false);
        }
        let directory = self.root().join("narrative/recovery");
        if std::fs::symlink_metadata(&directory).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error::Malformed {
                path: directory,
                reason: "Recovery storage cannot be a symbolic link.".into(),
            });
        }
        std::fs::create_dir_all(&directory).map_err(|e| Error::io(&directory, e))?;
        let recovery = directory.join(format!("{}.{}.deleted", deletion.id, wobu_core::new_id()));
        let placeholder = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&recovery)
            .map_err(|e| Error::io(&recovery, e))?;
        drop(placeholder);
        match std::fs::rename(&target, &recovery) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::remove_file(&recovery).map_err(|e| Error::io(&recovery, e))?;
                return Ok(false);
            }
            Err(e) => return Err(Error::io(&target, e)),
        }
        if !std::fs::symlink_metadata(&recovery)
            .map_err(|e| Error::io(&recovery, e))?
            .file_type()
            .is_file()
        {
            return Err(Error::Malformed {
                path: recovery,
                reason: "Recovery captured an unsafe non-file source; no bytes were followed."
                    .into(),
            });
        }
        let (captured, actual) = atomic::read_stamped(&recovery)?
            .ok_or_else(|| Error::NoSuchNode(deletion.target.clone()))?;
        if actual.hash != deletion.hash {
            // A writer won between read and move. Restore without clobbering a
            // further winner; either way its full bytes remain in recovery.
            atomic::guarded_write(self.root(), &target, &captured, None, &self.peer)?;
            self.index_narrative_path(&deletion.target)?;
            return Ok(false);
        }
        self.index_narrative_path(&deletion.target)?;
        Ok(true)
    }
}
