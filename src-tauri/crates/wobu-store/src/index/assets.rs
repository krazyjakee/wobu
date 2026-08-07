//! Assets and the links that attach them to nodes.

use rusqlite::{OptionalExtension, params};
use wobu_core::{Asset, AssetLink, AssetRole, Id};

use super::*;
use crate::error::Result;

impl Index {
    /// Record a blob.
    ///
    /// `INSERT OR REPLACE` rather than an upsert on `id`, because there are
    /// three unique columns here and any of them can be the one that collides:
    /// a file re-imported after being renamed on disk keeps its id and changes
    /// its `rel_path`. Replacing outright is safe precisely because no other
    /// table references this one.
    pub fn upsert_asset(&self, asset: &Asset) -> Result<()> {
        self.conn.execute(
            UPSERT_ASSET_SQL,
            params![
                asset.id.to_string(),
                asset.hash,
                serde_json::to_value(asset.kind)?.as_str().unwrap_or("reference"),
                asset.rel_path,
                asset.thumb_path,
                asset.mime,
                asset.width,
                asset.height,
                asset.bytes as i64,
                asset.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Newest first — the library is browsed as "what did I just bring in",
    /// not alphabetically by hash, which is an order with no meaning at all.
    pub fn list_assets(&self) -> Result<Vec<Asset>> {
        let mut stmt = self.conn.prepare(&format!("{ASSET_COLUMNS} ORDER BY created_at DESC"))?;
        let rows = stmt.query_map([], asset_row)?;
        Ok(rows.filter_map(std::result::Result::ok).flatten().collect())
    }

    pub fn asset(&self, id: Id) -> Result<Option<Asset>> {
        Ok(self
            .conn
            .query_row(
                &format!("{ASSET_COLUMNS} WHERE id = ?1"),
                params![id.to_string()],
                asset_row,
            )
            .optional()?
            .flatten())
    }

    /// Every asset path the index holds, for the reconcile that compares a
    /// directory listing against it.
    pub fn asset_paths(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT rel_path FROM assets")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn remove_asset_by_rel_path(&self, rel_path: &str) -> Result<()> {
        // Only the description of the blob. Any `asset_links` row pointing at
        // it stays: the file may be back in a moment — a share reconnecting, a
        // sync client catching up — and dropping the links would quietly strip
        // the roles out of somebody's node while their folder was mid-sync.
        self.conn.execute("DELETE FROM assets WHERE rel_path = ?1", params![rel_path])?;
        Ok(())
    }

    /* ── generations ────────────────────────────────────────────────────
     *
     * Canonically one immutable JSON document per row. The columns here are a
     * read model for the Concepts grid; deleting every one and scanning the
     * month shards produces exactly the same rows again.
     */

    /// Every asset attached to a node, strongest first.
    ///
    /// Ordered by weight because every caller is either drawing a strip of
    /// references or filling a budget, and both want the ones that matter most
    /// first. Ties break on role so the order is stable between calls rather
    /// than left to SQLite's row order.
    pub fn asset_links_of(&self, node_id: Id) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE node_id = ?1 ORDER BY weight DESC, role, asset_id",
        )?;
        collect_asset_links(stmt.query_map(params![node_id.to_string()], asset_link_row)?)
    }

    /// The same, narrowed to one role — what the per-role image budget asks.
    ///
    /// Filtered in SQL rather than by the caller so that the covering index on
    /// `(node_id, role)` is what does the work; a budget that pulled every
    /// reference on a node and then discarded six sevenths of them would make
    /// the index pointless.
    pub fn asset_links_in_role(&self, node_id: Id, role: AssetRole) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE node_id = ?1 AND role = ?2 ORDER BY weight DESC, asset_id",
        )?;
        collect_asset_links(
            stmt.query_map(params![node_id.to_string(), role.as_str()], asset_link_row)?,
        )
    }

    /// Every node using this asset — "3 characters reference this".
    ///
    /// Also the honest answer to "is anything still using this file", which is
    /// a question the library will want to ask. It is deliberately *not* wired
    /// to deletion: an asset with no links is still a file the user imported.
    pub fn asset_backlinks(&self, asset_id: Id) -> Result<Vec<AssetLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, asset_id, role, weight, enabled FROM asset_links
             WHERE asset_id = ?1 ORDER BY node_id, role",
        )?;
        collect_asset_links(stmt.query_map(params![asset_id.to_string()], asset_link_row)?)
    }

    /// A node's cover image, without opening its file.
    ///
    /// A column rather than a read because the surfaces that want it — the
    /// Launcher's project cards, any grid of tiles — want one per node at once,
    /// and the whole point of the index is that a card does not cost a file
    /// read over SMB.
    pub fn cover_asset_of(&self, node_id: Id) -> Result<Option<Id>> {
        let raw: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT cover_asset_id FROM nodes WHERE id = ?1",
                params![node_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(raw.flatten().and_then(|s| Id::from_string(&s).ok()))
    }

    /* ── corrupt files ───────────────────────────────────────────────────
     *
     * A sync client copying a half-written file is the expected cause, and a
     * truncated YAML frontmatter block the expected shape. Three rules follow,
     * and all three are about *not* acting:
     *
     * - the file is never written over, so the user's bytes survive;
     * - its node row is never removed, because a live row next to a broken
     *   file is how the user finds their data again;
     * - the parse error is kept verbatim, because "expected a mapping at line
     *   4" is the only thing that tells them what to fix.
     */
}
