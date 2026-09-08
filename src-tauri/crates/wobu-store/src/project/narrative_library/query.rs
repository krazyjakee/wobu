use super::*;
use crate::narrative::library::{Counts, Match, Projection, Summary};
use serde::{Deserialize, Serialize};
use wobu_narrative::{NamedClassification, WorldDocument};

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("Scene library changed. Restart the results to see a consistent page.")]
    StaleRevision,
    #[error("Invalid Scene library query: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Store(#[from] Error),
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryQuery {
    pub query: String,
    pub participant: String,
    pub quest: String,
    pub act: String,
    pub arc: String,
    pub tag: String,
    pub policy: String,
    pub review: String,
    pub freshness: String,
    pub missing: bool,
    pub include_drafts: bool,
    pub sort: String,
    pub offset: usize,
    pub limit: usize,
    pub revision: Option<String>,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub unreadable_offset: usize,
}
impl Default for LibraryQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            participant: String::new(),
            quest: String::new(),
            act: String::new(),
            arc: String::new(),
            tag: String::new(),
            policy: String::new(),
            review: String::new(),
            freshness: String::new(),
            missing: false,
            include_drafts: false,
            sort: "name".into(),
            offset: 0,
            limit: 25,
            revision: None,
            ids: vec![],
            unreadable_offset: 0,
        }
    }
}
impl LibraryQuery {
    fn validate(&self) -> std::result::Result<(), QueryError> {
        if self.query.chars().count() > 500
            || self.limit == 0
            || self.limit > 100
            || self.offset > 1_000_000
            || self.unreadable_offset > 1_000_000
            || self.ids.len() > 16
        {
            return Err(QueryError::Invalid(
                "search<=500 characters, page size1–100, offset<=1000000, at most16 lookup IDs",
            ));
        }
        for id in [&self.participant, &self.quest, &self.act, &self.arc, &self.tag]
            .into_iter()
            .chain(self.ids.iter())
        {
            if !id.is_empty()
                && id.parse::<wobu_core::Id>().ok().is_none_or(|value| value.to_string() != *id)
            {
                return Err(QueryError::Invalid("use canonical stable IDs"));
            }
        }
        if !["", "generated", "edited", "locked"].contains(&self.policy.as_str())
            || !["", "draft", "approved"].contains(&self.review.as_str())
            || !["", "current", "out_of_date"].contains(&self.freshness.as_str())
            || !["name", "nameDescending"].contains(&self.sort.as_str())
        {
            return Err(QueryError::Invalid("unknown filter or sort"));
        }
        if self.revision.as_ref().is_some_and(|value| !narrative::registry::valid_hash(value)) {
            return Err(QueryError::Invalid("invalid results revision"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub id: String,
    pub name: String,
    pub missing: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    pub summary: Summary,
    pub act: Option<Label>,
    pub arc: Option<Label>,
    pub tags: Vec<Label>,
    pub quests: Vec<Label>,
    pub participants: Vec<String>,
    pub slots: usize,
    pub filled: usize,
    pub beats: usize,
    pub counts: Counts,
    pub matches: Vec<Match>,
    pub match_count: usize,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub participants: Vec<String>,
    pub acts: Vec<Label>,
    pub arcs: Vec<Label>,
    pub tags: Vec<Label>,
    pub quests: Vec<Label>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unreadable {
    pub rel: String,
    pub reason: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage {
    pub revision: String,
    pub total: usize,
    pub scene_count: usize,
    pub rows: Vec<LibraryRow>,
    pub unreadable: Vec<Unreadable>,
    pub unreadable_total: usize,
    pub unreadable_next_offset: Option<usize>,
    pub missing_ids: Vec<String>,
    pub facets: Facets,
}
fn labels(records: &[NamedClassification]) -> Vec<Label> {
    let mut result = records
        .iter()
        .map(|r| Label { id: r.id.to_string(), name: r.name.clone(), missing: false })
        .collect::<Vec<_>>();
    result.sort_by_key(|r| (r.name.to_lowercase(), r.id.clone()));
    result
}
fn label(id: &str, records: &[Label], kind: &str) -> Label {
    records.iter().find(|r| r.id == id).cloned().unwrap_or_else(|| Label {
        id: id.into(),
        name: format!("Missing {kind} ({id})"),
        missing: true,
    })
}
impl Project {
    pub fn library_query(
        &self,
        query: &LibraryQuery,
    ) -> std::result::Result<LibraryPage, QueryError> {
        query.validate()?;
        let inventory = self.scene_inventory()?;
        let world_file = self.world_document()?;
        let world = world_file.as_ref().map(|(w, _)| w.clone()).unwrap_or_default();
        let world_hash = world_file.as_ref().map(|(_, s)| &s.hash);
        let revision = blake3::hash(format!("{}:{world_hash:?}", inventory.revision).as_bytes())
            .to_hex()
            .to_string();
        if query.revision.as_ref().is_some_and(|old| old != &revision) {
            return Err(QueryError::StaleRevision);
        }
        let projections = self
            .index
            .library_projections()?
            .into_iter()
            .filter(|p| !inventory.ambiguous.contains(&p.summary.id))
            .collect::<Vec<_>>();
        let scene_count = projections.len();
        let memberships = quest_memberships(&world);
        let facets = Facets {
            participants: projections
                .iter()
                .flat_map(|p| p.participants.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            acts: labels(&world.acts),
            arcs: labels(&world.arcs),
            tags: labels(&world.tags),
            quests: world
                .quests
                .iter()
                .map(|q| Label { id: q.id.to_string(), name: q.name.clone(), missing: false })
                .collect(),
        };
        let needle = query.query.trim().to_lowercase();
        let matching = (!needle.is_empty())
            .then(|| self.index.library_matches(&needle, query.include_drafts))
            .transpose()?;
        let lifecycle = (!query.policy.is_empty()
            || !query.review.is_empty()
            || !query.freshness.is_empty())
        .then(|| self.index.library_lifecycle(&query.policy, &query.review, &query.freshness))
        .transpose()?;
        let missing_ids = query
            .ids
            .iter()
            .filter(|id| !projections.iter().any(|p| p.summary.id == **id))
            .cloned()
            .collect();
        let mut selected = projections
            .into_iter()
            .filter(|p| {
                if !query.ids.is_empty() {
                    return query.ids.contains(&p.summary.id);
                }
                matching.as_ref().is_none_or(|m| m.contains(&p.summary.rel))
                    && lifecycle.as_ref().is_none_or(|m| m.contains(&p.summary.rel))
                    && (query.participant.is_empty() || p.participants.contains(&query.participant))
                    && (query.act.is_empty() || p.act_id.as_ref() == Some(&query.act))
                    && (query.arc.is_empty() || p.arc_id.as_ref() == Some(&query.arc))
                    && (query.tag.is_empty() || p.tag_ids.contains(&query.tag))
                    && (query.quest.is_empty()
                        || memberships
                            .get(&p.summary.id)
                            .is_some_and(|qs| qs.iter().any(|q| q.id == query.quest)))
                    && (!query.missing || p.filled < p.slots)
            })
            .collect::<Vec<_>>();
        selected.sort_by_key(|p| (p.summary.name.to_lowercase(), p.summary.id.clone()));
        if query.sort == "nameDescending" {
            selected.reverse()
        }
        let total = selected.len();
        let mut rows = Vec::new();
        for p in selected.into_iter().skip(query.offset).take(query.limit) {
            let (match_count, matches) = if needle.is_empty() || !query.ids.is_empty() {
                (0, vec![])
            } else {
                self.index.library_snippets(&p.summary.rel, &needle, query.include_drafts)?
            };
            rows.push(row(p, &facets, &memberships, match_count, matches));
        }
        let unreadable_total = inventory.catalog.unreadable.len();
        let unreadable = inventory
            .catalog
            .unreadable
            .into_iter()
            .skip(query.unreadable_offset)
            .take(query.limit)
            .map(|u| Unreadable { rel: u.rel, reason: u.reason.chars().take(500).collect() })
            .collect();
        let unreadable_next_offset = (query.unreadable_offset.saturating_add(query.limit)
            < unreadable_total)
            .then_some(query.unreadable_offset + query.limit);
        // The same content inventory is checked again before returning a page.
        // A changing corpus is an explicit restart, never mixed cached authority.
        if self.scene_inventory()?.revision != inventory.revision
            || self.world_document()?.as_ref().map(|(_, s)| &s.hash) != world_hash
        {
            return Err(QueryError::StaleRevision);
        }
        Ok(LibraryPage {
            revision,
            total,
            scene_count,
            rows,
            unreadable,
            unreadable_total,
            unreadable_next_offset,
            missing_ids,
            facets,
        })
    }
}
fn quest_memberships(world: &WorldDocument) -> BTreeMap<String, Vec<Label>> {
    let mut map = BTreeMap::<String, Vec<Label>>::new();
    for quest in &world.quests {
        for id in quest.scene_ids.iter().collect::<BTreeSet<_>>() {
            map.entry(id.to_string()).or_default().push(Label {
                id: quest.id.to_string(),
                name: quest.name.clone(),
                missing: false,
            });
        }
    }
    map
}
fn row(
    p: Projection,
    facets: &Facets,
    memberships: &BTreeMap<String, Vec<Label>>,
    match_count: usize,
    matches: Vec<Match>,
) -> LibraryRow {
    LibraryRow {
        act: p.act_id.as_ref().map(|id| label(id, &facets.acts, "act")),
        arc: p.arc_id.as_ref().map(|id| label(id, &facets.arcs, "arc")),
        tags: p.tag_ids.iter().map(|id| label(id, &facets.tags, "tag")).collect(),
        quests: memberships.get(&p.summary.id).cloned().unwrap_or_default(),
        summary: p.summary,
        participants: p.participants,
        slots: p.slots,
        filled: p.filled,
        beats: p.beats,
        counts: p.counts,
        matches,
        match_count,
    }
}
