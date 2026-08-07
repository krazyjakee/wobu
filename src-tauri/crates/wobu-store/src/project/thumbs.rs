//! Thumbnails, and deciding when it is this machine's job to make one.
//!
//! Generating is skipped on a read-only or network-shared project: the cost is
//! paid once by whoever owns the folder, not by every peer that opens it.

use std::collections::HashSet;

use wobu_core::Id;

use super::*;
use crate::error::{Error, Result};
use crate::scan::Cancel;

impl Project {
    /// Every blob the index has no thumbnail recorded against.
    ///
    /// What a project that arrived over sync hands to
    /// [`thumbs::ensure_all`](crate::thumbs::ensure_all). Read out in one go and
    /// returned by value on purpose: the pass that follows takes seconds to
    /// minutes and must run with nothing of this project's held, which is the
    /// same rule the shell's project mutex is built on.
    ///
    /// Filtered from the index rather than from a directory listing, because
    /// `assets::describe_at` already stats the thumbnail when it describes a
    /// blob — so this costs one local SQLite read where the honest-looking
    /// version costs one round trip per asset over SMB.
    pub fn missing_thumbs(&self) -> Result<Vec<crate::thumbs::ThumbTarget>> {
        Ok(self
            .index
            .list_assets()?
            .into_iter()
            .filter(|asset| asset.thumb_path.is_none())
            .map(|asset| crate::thumbs::ThumbTarget {
                asset_id: asset.id,
                hash: asset.hash,
                rel_path: asset.rel_path,
            })
            .collect())
    }

    /// The immutable facts needed to draw one asset's thumbnail.
    ///
    /// Returned by value so source I/O and pixel work can happen after the
    /// project mutex has been released.
    pub fn thumb_target(&self, id: Id) -> Result<Option<crate::thumbs::ThumbTarget>> {
        Ok(self.index.asset(id)?.map(|asset| crate::thumbs::ThumbTarget {
            asset_id: asset.id,
            hash: asset.hash,
            rel_path: asset.rel_path,
        }))
    }

    /// The bounded set of thumbnail inputs requested by one visible history page.
    pub fn thumb_targets(&self, ids: &[Id]) -> Result<Vec<crate::thumbs::ThumbTarget>> {
        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        for id in ids {
            if !seen.insert(*id) {
                continue;
            }
            if let Some(target) = self.thumb_target(*id)? {
                targets.push(target);
            }
        }
        Ok(targets)
    }

    /// Whether a thumbnail missing from disk may be written right now.
    ///
    /// Existing thumbnails remain usable in read-only projects, so callers
    /// carry this answer beside the target rather than rejecting the request.
    pub fn can_write_thumb(&self) -> bool {
        self.ensure_writable().is_ok()
    }

    /// Record thumbnail results already proved present outside the shell lock.
    ///
    /// Asset identity is rechecked in the local index before mutation. There is
    /// intentionally no filesystem access here: checking immediately before
    /// this call offers no stronger guarantee against a collaborator deleting
    /// a disposable thumbnail immediately after it, and would block every
    /// index query on a share stat for each result.
    pub fn record_thumb_targets(&mut self, targets: &[crate::thumbs::ThumbTarget]) -> Result<()> {
        for target in targets {
            let Some(mut asset) = self.index.asset(target.asset_id)? else { continue };
            if asset.hash != target.hash || asset.rel_path != target.rel_path {
                continue;
            }
            let rel = crate::thumbs::rel_path(&asset.hash);
            if asset.thumb_path.as_deref() == Some(rel.as_str()) {
                continue;
            }
            asset.thumb_path = Some(rel);
            self.index.upsert_asset(&asset)?;
        }
        Ok(())
    }

    /// Make one blob's thumbnail if the folder has not got one, and record it.
    ///
    /// The lazy half of #25, and the one a grid tile reaches for. `Ok(None)`
    /// covers all three ways there is legitimately no thumbnail and never will
    /// be one right now — no such asset, a folder nothing can be written into,
    /// and a blob whose pixels will not decode — because each of those is a tile
    /// that falls back to a placeholder rather than an error the user can act
    /// on. Anything else is a real failure and is reported.
    pub fn ensure_thumb(&mut self, id: Id, cancel: &Cancel) -> Result<Option<String>> {
        let Some(asset) = self.index.asset(id)? else { return Ok(None) };

        // Only when one has to be *made*. A read-only share that already holds
        // thumbnails — the ordinary way a folder is published — still serves
        // them, and refusing here would blank the grid on exactly that project.
        if !crate::thumbs::exists(&self.root, &asset.hash) && self.ensure_writable().is_err() {
            return Ok(None);
        }

        match crate::thumbs::ensure(&self.root, &asset.hash, &asset.rel_path, cancel) {
            Ok(thumb) => {
                self.record_thumbs(&[id])?;
                Ok(Some(thumb.rel_path))
            }
            Err(Error::Undecodable { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Note that these blobs now have thumbnails in the folder.
    ///
    /// Every id is re-checked against the disk rather than trusted, because the
    /// caller of the bulk pass is holding a list assembled before it ran and the
    /// index is the thing the UI reads: a row claiming a thumbnail that is not
    /// there is a broken image in the grid, which is worse than the null it
    /// replaced.
    pub fn record_thumbs(&mut self, ids: &[Id]) -> Result<()> {
        for id in ids {
            let Some(mut asset) = self.index.asset(*id)? else { continue };
            let rel = crate::thumbs::rel_path(&asset.hash);
            if !crate::thumbs::exists(&self.root, &asset.hash) {
                continue;
            }
            if asset.thumb_path.as_deref() == Some(rel.as_str()) {
                continue;
            }
            asset.thumb_path = Some(rel);
            self.index.upsert_asset(&asset)?;
        }
        Ok(())
    }
}
