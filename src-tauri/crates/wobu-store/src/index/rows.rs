//! Moving between SQLite rows and domain values.
//!
//! Kept apart from the queries so a column list and the code that reads or
//! writes it sit together — the failure this prevents is a statement gaining a
//! column and its counterpart still working by the old index.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use wobu_core::{
    Asset, AssetKind, AssetLink, AssetRole, DescriptionState, Generation, Id, LinkEdge, NodeKind,
    NodeSummary,
};

use super::*;
use crate::error::Result;

/// Turn whatever the user typed into an FTS5 `MATCH` expression, or `None` when
/// there is nothing worth searching for.
///
/// Two things are going on here, and they pull in opposite directions.
///
/// **Every term is quoted**, so that FTS5 operators the user did not mean — a
/// stray `-`, `*`, `"`, or the word `AND` — cannot become a syntax error. The
/// palette searches on every keystroke, so a query is malformed far more often
/// than it is complete.
///
/// **But the terms are joined with `AND` rather than quoted as one phrase.**
/// Quoting the whole query makes it an adjacency test, so "scarred enforcer"
/// misses notes reading "scarred ex-guild enforcer" — which is exactly the
/// query someone types when half-remembering a phrase, and exactly the case
/// searching notes exists to serve. Each term keeps its `*` so prefixes still
/// match while the word is still being typed.
///
/// Terms with no alphanumeric character are dropped rather than escaped: `"-"`
/// tokenizes to nothing, and a query made only of those should find nothing
/// rather than error.
pub(super) fn fts_match_expr(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() { None } else { Some(terms.join(" AND ")) }
}

/// Raw navigator columns, decoded once and converted separately so a row from
/// a newer build can be skipped without turning a disposable-index mismatch
/// into a query failure.
pub(super) struct NodeSummaryRow {
    id: String,
    kind: String,
    name: String,
    slug: String,
    summary: String,
    parent: Option<String>,
    state: String,
}

impl NodeSummaryRow {
    pub(super) fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            slug: row.get(3)?,
            summary: row.get(4)?,
            parent: row.get(5)?,
            state: row.get(6)?,
        })
    }

    pub(super) fn into_summary(self, stale: &BTreeSet<Id>) -> Option<NodeSummary> {
        let id = Id::from_string(&self.id).ok()?;
        let kind = self.kind.parse::<NodeKind>().ok()?;
        let description_state = if stale.contains(&id) {
            DescriptionState::Stale
        } else {
            serde_json::from_value(serde_json::Value::String(self.state)).unwrap_or_default()
        };
        Some(NodeSummary {
            id,
            kind,
            name: self.name,
            slug: self.slug,
            summary: self.summary,
            parent_id: self.parent.and_then(|parent| Id::from_string(&parent).ok()),
            description_state,
        })
    }
}

pub(super) struct StalenessRow {
    pub(super) id: String,
    pub(super) state: String,
    pub(super) subject: String,
    pub(super) source: String,
    pub(super) stamp: String,
}

impl StalenessRow {
    pub(super) fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            state: row.get(1)?,
            subject: row.get(2)?,
            source: row.get(3)?,
            stamp: row.get(4)?,
        })
    }
}

pub(super) fn document_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    row.get(0)
}

pub(super) fn generation_prompt_excerpt(prompt: &str) -> String {
    const LIMIT: usize = 240;
    let mut excerpt: String = prompt.chars().take(LIMIT).collect();
    if prompt.chars().count() > LIMIT {
        excerpt.push('…');
    }
    excerpt
}

pub(super) fn generation_scene_names(generation: &Generation) -> Result<String> {
    let names = generation
        .params
        .get("sceneComposition")
        .and_then(|scene| scene.get("subjectNames"))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(serde_json::to_string(&names)?)
}

pub(super) fn generation_summary_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Option<GenerationSummary>> {
    let id = row.get::<_, String>(0)?;
    let node_id = row.get::<_, String>(1)?;
    let created_at = row.get::<_, String>(2)?;
    let (Ok(id), Ok(node_id), Ok(created_at)) = (
        Id::from_string(&id),
        Id::from_string(&node_id),
        DateTime::parse_from_rfc3339(&created_at),
    ) else {
        return Ok(None);
    };
    let first_asset_id =
        row.get::<_, Option<String>>(9)?.and_then(|value| Id::from_string(&value).ok());
    let scene_subject_names =
        serde_json::from_str(&row.get::<_, String>(13)?).unwrap_or_else(|_| Vec::new());
    Ok(Some(GenerationSummary {
        id,
        node_id,
        created_at: created_at.with_timezone(&Utc),
        preset: row.get(3)?,
        view_type: row.get(4)?,
        backend: row.get(5)?,
        model: row.get(6)?,
        seed: row.get::<_, i64>(7)? as u64,
        prompt_excerpt: row.get(8)?,
        first_asset_id,
        output_count: row.get::<_, i64>(10)? as u32,
        seed_source: row.get(11)?,
        used_locked_seed: row.get::<_, Option<i32>>(12)?.map(|value| value != 0),
        scene_subject_names,
        thumbnail_path: row.get(14)?,
    }))
}

pub(super) fn collect_json_documents<T, I>(rows: I) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
    I: Iterator<Item = rusqlite::Result<String>>,
{
    let mut out = Vec::new();
    for row in rows {
        if let Ok(document) = serde_json::from_str::<T>(&row?) {
            out.push(document);
        }
    }
    Ok(out)
}

pub(super) struct LinkRow {
    from: String,
    to: String,
    role: String,
    weight: f32,
    enabled: i32,
}

impl LinkRow {
    pub(super) fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            from: row.get(0)?,
            to: row.get(1)?,
            role: row.get(2)?,
            weight: row.get(3)?,
            enabled: row.get(4)?,
        })
    }

    pub(super) fn into_edge(self) -> Option<LinkEdge> {
        let from_id = Id::from_string(&self.from).ok()?;
        let to_id = Id::from_string(&self.to).ok()?;
        let role = serde_json::from_value(serde_json::Value::String(self.role)).ok()?;
        Some(LinkEdge { from_id, to_id, role, weight: self.weight, enabled: self.enabled != 0 })
    }
}

pub(super) fn collect_links<I>(rows: I) -> Result<Vec<LinkEdge>>
where
    I: Iterator<Item = rusqlite::Result<LinkRow>>,
{
    let mut out = Vec::new();
    for row in rows {
        if let Some(edge) = row?.into_edge() {
            out.push(edge);
        }
    }
    Ok(out)
}

pub(super) const ASSET_COLUMNS: &str =
    "SELECT id, hash, kind, rel_path, thumb_path, mime, width, height,
                                    bytes, created_at
                             FROM assets";

/// One row as an [`Asset`], or `None` for a row this build cannot make sense
/// of. The same forgiveness [`Index::list_nodes`] shows, for the same reason: a
/// single unreadable row must not take the whole library off the screen.
pub(super) fn asset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Option<Asset>> {
    let (id, kind, created) =
        (r.get::<_, String>(0)?, r.get::<_, String>(2)?, r.get::<_, String>(9)?);
    let (Ok(id), Ok(kind), Ok(created_at)) = (
        Id::from_string(&id),
        serde_json::from_value::<AssetKind>(serde_json::Value::String(kind)),
        DateTime::parse_from_rfc3339(&created),
    ) else {
        return Ok(None);
    };
    Ok(Some(Asset {
        id,
        hash: r.get(1)?,
        kind,
        rel_path: r.get(3)?,
        thumb_path: r.get(4)?,
        mime: r.get(5)?,
        width: r.get(6)?,
        height: r.get(7)?,
        bytes: r.get::<_, i64>(8)? as u64,
        created_at: created_at.with_timezone(&Utc),
    }))
}

/// One `asset_links` row as the tuple the collector parses.
pub(super) fn asset_link_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, String, f32, i32)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

/// Rows to [`AssetLink`]s, skipping any this build cannot parse.
///
/// The same forgiveness the node and asset readers show, and the same reason: a
/// role written by a newer Wobu must cost the user that one reference, not the
/// whole strip.
pub(super) fn collect_asset_links<I>(rows: I) -> Result<Vec<AssetLink>>
where
    I: Iterator<Item = rusqlite::Result<(String, String, String, f32, i32)>>,
{
    let mut out = Vec::new();
    for row in rows {
        let (node, asset, role, weight, enabled) = row?;
        let (Ok(node_id), Ok(asset_id)) = (Id::from_string(&node), Id::from_string(&asset)) else {
            continue;
        };
        let Ok(role) = serde_json::from_value::<AssetRole>(serde_json::Value::String(role)) else {
            continue;
        };
        out.push(AssetLink { asset_id, node_id, role, weight, enabled: enabled != 0 });
    }
    Ok(out)
}

/* ── versions ─────────────────────────────────────────────────────────────
 *
 * A "version" is a short content hash of the bytes an enhance actually reads.
 * `wobu_core::EnhanceStamp` argues why it is a hash and not a timestamp; these
 * two decide *which* bytes, and what is left out matters as much as what is in.
 */

/// One `generations` row, derived once.
///
/// Two statements write this row — the single upsert and the rebuild's prepared
/// one — and they had a copy each of the same nineteen bindings. A column added
/// to one and not the other is a row that means different things depending on
/// which path wrote it, which is the kind of thing that only shows up as a
/// receipt the gallery cannot page.
pub(super) struct GenerationRow<'a> {
    generation: &'a Generation,
    rel_path: &'a str,
    stamp: &'a Stamp,
    id: String,
    node_id: String,
    created_at: String,
    seed: i64,
    excerpt: String,
    first_asset: Option<String>,
    outputs: i64,
    seed_source: Option<&'a str>,
    used_locked_seed: Option<i32>,
    scene_names: String,
    size: i64,
    document: String,
}

impl<'a> GenerationRow<'a> {
    pub(super) fn new(
        generation: &'a Generation,
        rel_path: &'a str,
        stamp: &'a Stamp,
    ) -> Result<GenerationRow<'a>> {
        Ok(GenerationRow {
            generation,
            rel_path,
            stamp,
            id: generation.id.to_string(),
            node_id: generation.node_id.to_string(),
            created_at: generation.created_at.to_rfc3339(),
            seed: generation.seed as i64,
            excerpt: generation_prompt_excerpt(&generation.compiled_prompt),
            first_asset: generation.output_asset_ids.first().map(ToString::to_string),
            outputs: generation.output_asset_ids.len() as i64,
            seed_source: generation.params.get("seedSource").and_then(serde_json::Value::as_str),
            used_locked_seed: generation
                .params
                .get("usedLockedSeed")
                .and_then(serde_json::Value::as_bool)
                .map(i32::from),
            scene_names: generation_scene_names(generation)?,
            size: stamp.size as i64,
            document: serde_json::to_string(generation)?,
        })
    }

    /// In the column order of [`super::UPSERT_GENERATION_SQL`].
    pub(super) fn params(&self) -> [&dyn rusqlite::ToSql; 19] {
        [
            &self.id,
            &self.node_id,
            &self.created_at,
            &self.generation.preset,
            &self.generation.view_type,
            &self.generation.backend,
            &self.generation.model,
            &self.seed,
            &self.excerpt,
            &self.first_asset,
            &self.outputs,
            &self.seed_source,
            &self.used_locked_seed,
            &self.scene_names,
            &self.rel_path,
            &self.stamp.mtime_ms,
            &self.size,
            &self.stamp.hash,
            &self.document,
        ]
    }
}
