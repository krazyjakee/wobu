use super::Index;
use crate::{
    NarrativeIndexEntry, Result,
    narrative::{
        library::{self, Match, Projection},
        registry::NarrativeFileKind,
    },
};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

pub(super) fn remove(connection: &Connection, rel: &str) -> Result<()> {
    for table in ["narrative_scene_summary", "narrative_scene_text", "narrative_scene_variant"] {
        connection.execute(&format!("DELETE FROM {table} WHERE rel=?1"), [rel])?;
    }
    Ok(())
}
pub(super) fn put(connection: &Connection, entry: &NarrativeIndexEntry) -> Result<()> {
    if entry.kind != NarrativeFileKind::Scene {
        return Ok(());
    }
    remove(connection, &entry.rel)?;
    let parsed = entry
        .document
        .as_ref()
        .map(|doc| serde_json::from_value::<wobu_narrative::SceneDocument>(doc.clone()))
        .transpose()?;
    let projected = parsed.as_ref().map(|doc| library::project(&doc.scene, &entry.rel));
    let json = projected.as_ref().map(|(p, _, _)| serde_json::to_string(p)).transpose()?;
    connection.execute("INSERT INTO narrative_scene_summary(rel,hash,scene_id,name,projection,error) VALUES(?1,?2,?3,?4,?5,?6)", params![entry.rel,entry.hash,entry.id,entry.name,json,entry.error])?;
    if let Some((_, texts, variants)) = projected {
        let mut insert = connection
            .prepare_cached("INSERT INTO narrative_scene_text VALUES(?1,?2,?3,?4,?5,?6)")?;
        for (ordinal, text) in texts.iter().enumerate() {
            insert.execute(params![
                entry.rel,
                ordinal as i64,
                text.text,
                text.text.to_lowercase(),
                serde_json::to_string(&text.target)?,
                text.target.draft
            ])?;
        }
        let mut insert = connection
            .prepare_cached("INSERT INTO narrative_scene_variant VALUES(?1,?2,?3,?4,?5)")?;
        for (ordinal, variant) in variants.iter().enumerate() {
            insert.execute(params![
                entry.rel,
                ordinal as i64,
                variant.policy,
                variant.review,
                variant.freshness
            ])?;
        }
    }
    Ok(())
}
#[derive(Clone)]
pub(crate) struct Locator {
    pub rel: String,
    pub hash: String,
    pub id: Option<String>,
    pub name: String,
    pub error: Option<String>,
}
impl Index {
    pub(crate) fn scene_locators(&self) -> Result<Vec<Locator>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT rel,hash,scene_id,name,error FROM narrative_scene_summary ORDER BY rel",
        )?;
        Ok(statement
            .query_map([], |r| {
                Ok(Locator {
                    rel: r.get(0)?,
                    hash: r.get(1)?,
                    id: r.get(2)?,
                    name: r.get(3)?,
                    error: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub(crate) fn library_projections(&self) -> Result<Vec<Projection>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT projection FROM narrative_scene_summary WHERE projection IS NOT NULL",
        )?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub(crate) fn library_matches(
        &self,
        query: &str,
        include_drafts: bool,
    ) -> Result<BTreeSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT DISTINCT rel FROM narrative_scene_text WHERE instr(folded,?1)>0 AND (?2 OR draft=0)")?;
        Ok(statement
            .query_map(params![query, include_drafts], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub(crate) fn library_lifecycle(
        &self,
        policy: &str,
        review: &str,
        freshness: &str,
    ) -> Result<BTreeSet<String>> {
        let mut statement = self.conn.prepare_cached("SELECT DISTINCT rel FROM narrative_scene_variant WHERE (?1='' OR policy=?1) AND (?2='' OR review=?2) AND (?3='' OR freshness=?3)")?;
        Ok(statement
            .query_map(params![policy, review, freshness], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub(crate) fn library_snippets(
        &self,
        rel: &str,
        query: &str,
        include_drafts: bool,
    ) -> Result<(usize, Vec<Match>)> {
        let count:i64 = self.conn.query_row("SELECT COUNT(*) FROM narrative_scene_text WHERE rel=?1 AND instr(folded,?2)>0 AND (?3 OR draft=0)",params![rel,query,include_drafts],|row|row.get(0))?;
        let mut statement = self.conn.prepare_cached("SELECT text,target FROM narrative_scene_text WHERE rel=?1 AND instr(folded,?2)>0 AND (?3 OR draft=0) ORDER BY ordinal LIMIT 5")?;
        let matches = statement
            .query_map(params![rel, query, include_drafts], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|r| {
                let (text, target) = r?;
                let mut target: Match = serde_json::from_str(&target)?;
                // Folded byte positions cannot index the original string: İ, for
                // example, becomes two characters. Map through original scalars.
                let folded_offset = text.to_lowercase().find(query).unwrap_or(0);
                let mut folded_bytes = 0;
                let offset = text
                    .chars()
                    .position(|character| {
                        folded_bytes += character.to_lowercase().map(char::len_utf8).sum::<usize>();
                        folded_offset < folded_bytes
                    })
                    .unwrap_or(0);
                let start = offset.saturating_sub(40);
                let snippet: String = text.chars().skip(start).take(180).collect();
                target.snippet = format!(
                    "{}{}{}",
                    if start > 0 { "…" } else { "" },
                    snippet,
                    if text.chars().count() > start + 180 { "…" } else { "" }
                );
                Ok(target)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((count as usize, matches))
    }
}
