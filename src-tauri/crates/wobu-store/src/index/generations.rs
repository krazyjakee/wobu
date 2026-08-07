//! Generation receipts, and the paged view the gallery scrolls.
//!
//! The page is built in SQL rather than by loading every receipt and slicing:
//! a node with four thousand generations is a normal node, and a query that
//! reads all of them to return twenty is the one that makes it feel otherwise.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use wobu_core::{Generation, Id};

use super::*;
use crate::atomic::Stamp;
use crate::error::Result;

/// The indexed fields a concept tile needs. Full immutable receipts remain in
/// `generations.doc` and cross the bridge only when one tile is opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSummary {
    pub id: Id,
    pub node_id: Id,
    pub created_at: DateTime<Utc>,
    pub preset: String,
    pub view_type: Option<String>,
    pub backend: String,
    pub model: String,
    pub seed: u64,
    pub prompt_excerpt: String,
    pub first_asset_id: Option<Id>,
    pub output_count: u32,
    pub seed_source: Option<String>,
    pub used_locked_seed: Option<bool>,
    pub scene_subject_names: Vec<String>,
    /// Project-relative in the store; the command layer makes it absolute.
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationPageRequest {
    pub node_id: Option<Id>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationPage {
    pub items: Vec<GenerationSummary>,
    pub total: u64,
    pub next_offset: Option<u32>,
}

impl Index {
    pub fn upsert_generation(
        &self,
        generation: &Generation,
        rel_path: &str,
        stamp: &Stamp,
    ) -> Result<()> {
        let row = GenerationRow::new(generation, rel_path, stamp)?;
        self.conn.execute(UPSERT_GENERATION_SQL, &row.params()[..])?;
        Ok(())
    }

    /// One generation from local disk, without opening its canonical JSON.
    pub fn generation(&self, id: Id) -> Result<Option<Generation>> {
        let doc: Option<String> = self
            .conn
            .query_row(
                "SELECT doc FROM generations WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(doc.and_then(|doc| serde_json::from_str(&doc).ok()))
    }

    /// Whether any receipt still on the visible ledger claims this asset as an
    /// output.
    ///
    /// Assets are content-addressed, so two runs that produced identical bytes
    /// share one id and one file. Deleting one of those concepts must not take
    /// the picture out from under the other, which is why this asks the whole
    /// table rather than trusting the receipt being deleted. `first_asset_id`
    /// answers the common single-output case from a column; the `LIKE` widens
    /// it to multi-output receipts and the parse confirms the match, so a hit
    /// on the same id inside a prompt or an influence snapshot cannot pass for
    /// an output.
    pub fn generation_outputs_contain(&self, asset_id: Id) -> Result<bool> {
        let id = asset_id.to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT doc FROM generations WHERE first_asset_id = ?1 OR doc LIKE ?2")?;
        let candidates =
            collect_json_documents(stmt.query_map(params![id, format!("%{id}%")], document_row)?)?;
        Ok(candidates
            .iter()
            .any(|generation: &Generation| generation.output_asset_ids.contains(&asset_id)))
    }

    /// Full node receipts for the mesh adapter's immutable turnaround joins.
    pub fn generation_documents_for_node(&self, node_id: Id) -> Result<Vec<Generation>> {
        let mut stmt = self.conn.prepare(
            "SELECT doc FROM generations
             WHERE node_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        collect_json_documents(stmt.query_map(params![node_id.to_string()], document_row)?)
    }

    /// A bounded page of lightweight receipt rows, newest first.
    ///
    /// Every predicate is answered by an indexed/scalar column. The large JSON
    /// `doc` is deliberately absent from this query and is read only by
    /// [`generation`](Self::generation) after a person opens one receipt.
    pub fn generation_page(&self, request: &GenerationPageRequest) -> Result<GenerationPage> {
        let node_id = request.node_id.map(|id| id.to_string());
        let limit = request.limit.clamp(1, 100);
        let predicate = "(?1 IS NULL OR g.node_id = ?1)";
        let sql = format!(
            "SELECT g.id, g.node_id, g.created_at, g.preset, g.view_type,
                    g.backend, g.model, g.seed, g.prompt_excerpt,
                    g.first_asset_id, g.output_count, g.seed_source,
                    g.used_locked_seed, g.scene_subject_names, a.thumb_path
             FROM generations g
             LEFT JOIN assets a ON a.id = g.first_asset_id
             WHERE {predicate}
             ORDER BY g.created_at DESC, g.id DESC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows =
            stmt.query_map(params![node_id, limit, request.offset], generation_summary_row)?;
        let mut items = Vec::new();
        for row in rows {
            if let Some(summary) = row? {
                items.push(summary);
            }
        }

        let count_sql = format!("SELECT COUNT(*) FROM generations g WHERE {predicate}");
        let total =
            self.conn.query_row(&count_sql, params![node_id], |row| row.get::<_, i64>(0))? as u64;
        let consumed = request.offset.saturating_add(items.len() as u32);
        Ok(GenerationPage {
            items,
            total,
            next_offset: (u64::from(consumed) < total).then_some(consumed),
        })
    }

    /// Every generation path held by the disposable index.
    pub fn generation_paths(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT rel_path FROM generations")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn remove_generation_by_rel_path(&self, rel_path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM generations WHERE rel_path = ?1", params![rel_path])?;
        Ok(())
    }

    /* ── asset links ─────────────────────────────────────────────────────
     *
     * Canonically frontmatter, cached here. `upsert_node` is the only writer,
     * which is what makes these rows rebuildable: every path that reads a node
     * file — first scan, reconcile, save — already goes through it.
     */
}
