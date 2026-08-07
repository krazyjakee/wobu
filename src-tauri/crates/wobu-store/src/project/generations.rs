//! The generation log: what was asked for, what came back, what it cost.
//!
//! Append-only from the caller's side. A receipt is never edited after the
//! fact, because the whole point of it is to be the thing an image can be
//! reproduced from.

use std::path::PathBuf;

use wobu_core::{Generation, Id};

use super::*;
use crate::error::{Error, Result};
use crate::generations;

impl Project {
    /// Append one immutable generation record to the project.
    ///
    /// The JSON lands before the index row. If local SQLite is unavailable at
    /// that point the result is still safely recorded, and the next open or
    /// rebuild discovers it from the folder. There is deliberately no update
    /// counterpart: a retry or another collaborator's attempt gets a new ULID.
    pub fn record_generation(&mut self, generation: Generation) -> Result<Generation> {
        self.ensure_writable()?;
        if self.index.node(generation.node_id)?.is_none() {
            return Err(Error::NoSuchNode(generation.node_id.to_string()));
        }
        self.append_generation(generation)
    }

    /// Append a replay receipt even when its historical subject has since been
    /// deleted. Ordinary generation still requires a live node; this archival
    /// path is allowed only when `replayOf` names an immutable receipt already
    /// in this project.
    pub fn record_replay_generation(&mut self, generation: Generation) -> Result<Generation> {
        self.ensure_writable()?;
        let source = generation
            .params
            .get("replayOf")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Id::from_string(value).ok())
            .ok_or_else(|| Error::MalformedGeneration {
                path: PathBuf::from(generation.rel_path()),
                reason: "replay receipt has no valid replayOf id".to_string(),
            })?;
        if self.get_generation(source)?.is_none() {
            return Err(Error::NoSuchNode(format!("replay source {source}")));
        }
        self.append_generation(generation)
    }

    pub(super) fn append_generation(&mut self, generation: Generation) -> Result<Generation> {
        for asset_id in &generation.output_asset_ids {
            self.require_asset(*asset_id)?;
        }

        let (rel, stamp) = generations::write(&self.root, &generation)?;
        self.index.upsert_generation(&generation, &rel, &stamp)?;
        Ok(generation)
    }

    /// A bounded lightweight generation page for Concepts or History.
    pub fn generation_page(
        &self,
        request: &crate::GenerationPageRequest,
    ) -> Result<crate::GenerationPage> {
        self.index.generation_page(request)
    }

    /// Full node receipts for the mesh command's immutable turnaround join.
    pub fn list_generations(&self, node_id: Id) -> Result<Vec<Generation>> {
        self.index.generation_documents_for_node(node_id)
    }

    /// One indexed generation, without reading across the project share.
    pub fn get_generation(&self, id: Id) -> Result<Option<Generation>> {
        self.index.generation(id)
    }

    /// Remove a receipt from user-facing history while retaining it for spend accounting.
    ///
    /// The pictures go with it. A deleted concept that left its image in the
    /// Asset Library would only be deleted from the one view the user happened
    /// to be looking at, which is not what the button says.
    /// So each output blob is taken too — but only where nothing else claims
    /// it: an image pinned as a reference, chosen as a cover, or shared with a
    /// receipt that is still on the ledger stays exactly where it is, because
    /// removing it would break something the user never asked to touch.
    ///
    /// The receipt is archived first, and the outputs are cleaned up after it:
    /// deleting the concept is the operation the user asked for, so it is the
    /// one allowed to fail. An output that cannot be removed is left behind
    /// rather than turning a completed deletion into an error.
    pub fn delete_generation(&mut self, id: Id) -> Result<()> {
        self.ensure_writable()?;
        let generation =
            self.index.generation(id)?.ok_or_else(|| Error::NoSuchGeneration(id.to_string()))?;
        let rel = generations::archive(&self.root, &generation)?;
        self.index.remove_generation_by_rel_path(&rel)?;

        let mut seen = std::collections::BTreeSet::new();
        for asset_id in &generation.output_asset_ids {
            if seen.insert(*asset_id) {
                // A blob that will not go — an unreadable node file it cannot
                // clear itself against, a file held open by a viewer — is left
                // where it is. The concept has already been deleted, and
                // reporting a failure for work that succeeded would only invite
                // the user to try again at nothing.
                let _ = self.delete_unclaimed_output(*asset_id);
            }
        }
        Ok(())
    }

    /// Delete one output blob of a just-deleted receipt, if nothing claims it.
    ///
    /// Unlike [`delete_asset`](Self::delete_asset) a claim is not an error
    /// here: the user deleted a concept, not an image, and being told that the
    /// operation failed because they had pinned the result would be a refusal
    /// of a request that has already succeeded.
    pub(super) fn delete_unclaimed_output(&mut self, id: Id) -> Result<()> {
        let Some(asset) = self.get_asset(id)? else { return Ok(()) };
        if self.canonical_asset_users(id)? > 0 {
            return Ok(());
        }
        if self.index.generation_outputs_contain(id)? {
            return Ok(());
        }
        self.remove_asset_blob(&asset)
    }
}
