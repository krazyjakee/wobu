//! Raw, guarded repairs of scene files that cannot be represented as SceneFile.
use super::Project;
use crate::atomic::{Stamp, WriteOutcome};
use crate::{Error, Result, atomic, narrative};
use std::path::PathBuf;
use wobu_narrative::{SceneDocument, SceneId};

impl Project {
    /// Only a regular YAML file directly inside this project's scene directory.
    /// The editor receives catalog paths, never a general-purpose file API.
    pub fn scene_source_path(&self, rel: &str) -> Result<PathBuf> {
        let prefix = format!("{}/", narrative::SCENES_DIR);
        let name = rel.strip_prefix(&prefix).unwrap_or("");
        if name.is_empty()
            || name.contains(['/', '\\'])
            || !name.ends_with(".yaml")
            || name.contains(".conflict-")
        {
            return Err(Error::Malformed {
                path: self.root().to_path_buf(),
                reason: "Expected a canonical narrative/scenes/*.yaml file.".into(),
            });
        }
        let mut component = self.root().to_path_buf();
        for part in std::path::Path::new(rel).components() {
            component.push(part);
            if std::fs::symlink_metadata(&component)
                .map_err(|e| Error::io(&component, e))?
                .file_type()
                .is_symlink()
            {
                return Err(Error::Malformed {
                    path: component,
                    reason: "Scene paths cannot contain symbolic links.".into(),
                });
            }
        }
        let path = self.root().join(rel);
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
        let canonical = path.canonicalize().map_err(|e| Error::io(&path, e))?;
        let root = self.root().canonicalize().map_err(|e| Error::io(self.root(), e))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || !canonical.starts_with(root)
        {
            return Err(Error::Malformed {
                path,
                reason: "Scene source must be a regular file inside this project.".into(),
            });
        }
        Ok(path)
    }

    /// Repairs only parsed, supported source and retains the original bytes in
    /// an immutable recovery file. An intervening write goes to the standard
    /// conflict sibling; neither current source nor recovery is overwritten.
    pub fn repair_scene_source(
        &mut self,
        rel: &str,
        document: &SceneDocument,
        expected: &Stamp,
        expected_id: Option<SceneId>,
    ) -> Result<(narrative::SourceSave, String)> {
        self.ensure_writable()?;
        super::narrative_review::validate_manual(None, &document.scene)?;
        let _lock = super::narrative_review::scene_lock(self, document.scene.id)?;
        let path = self.scene_source_path(rel)?;
        let (previous, actual) =
            atomic::read_stamped(&path)?.ok_or_else(|| Error::NoSuchNode(rel.into()))?;
        if let Err(wobu_narrative::Error::UnsupportedSchemaVersion { .. }) =
            SceneDocument::parse(&previous)
        {
            return Err(Error::Malformed { path, reason: "This source uses an unsupported schema version. Open it in a compatible Wobu; repair cannot downgrade it.".into() });
        }
        let yaml = document
            .to_yaml()
            .map_err(|error| Error::Malformed { path: path.clone(), reason: error.to_string() })?;
        // Validate supported versions even if a non-command caller constructs a document.
        SceneDocument::parse(&yaml)
            .map_err(|error| Error::Malformed { path: path.clone(), reason: error.to_string() })?;
        if expected_id.is_some_and(|id| id != document.scene.id)
            || self
                .scene_catalog()?
                .scenes
                .iter()
                .any(|entry| entry.id == document.scene.id && entry.rel != rel)
        {
            return Err(Error::Malformed { path, reason: "Keep the original scene identity; that identity must not belong to another file.".into() });
        }
        let recovery_rel =
            format!("narrative/recovery/{}.{}.yaml", document.scene.id, expected.hash);
        if actual.hash == expected.hash {
            // Re-reading a now-valid file must not bypass normal editorial checks.
            if SceneDocument::parse(&previous).is_ok() {
                return Err(Error::Malformed {
                    path,
                    reason: "This file is valid now. Reload and use the normal source save.".into(),
                });
            }
            let directory = self.root().join("narrative/recovery");
            if std::fs::symlink_metadata(&directory)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(Error::Malformed {
                    path: directory,
                    reason: "Recovery storage cannot be a symbolic link.".into(),
                });
            }
            let recovery = self.root().join(&recovery_rel);
            match atomic::write_once(self.root(), &recovery, previous.as_bytes()) {
                Ok(_) => {}
                Err(Error::AlreadyExists(_))
                    if std::fs::symlink_metadata(&recovery).is_ok_and(|metadata| {
                        metadata.is_file() && !metadata.file_type().is_symlink()
                    }) && std::fs::read(&recovery).map_err(|e| Error::io(&recovery, e))?
                        == previous.as_bytes() => {}
                Err(error) => return Err(error),
            }
        }
        let saved =
            match atomic::guarded_write(self.root(), &path, &yaml, Some(expected), &self.peer)? {
                WriteOutcome::Written(stamp) => narrative::SourceSave::Saved(stamp),
                WriteOutcome::Conflict { conflict_path, .. } => narrative::SourceSave::Conflict {
                    conflict_path: crate::paths::to_rel_string(
                        conflict_path.strip_prefix(self.root()).unwrap_or(&conflict_path),
                    ),
                },
            };
        Ok((saved, recovery_rel))
    }
}
