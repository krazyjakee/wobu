//! The Node record — the one record type behind every entity in a project.

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::Id;
use crate::asset::AssetRef;
use crate::error::{Error, Result};
use crate::kind::{NodeKind, SectionValueKind, kind_def};
use crate::link::Link;

/// Lifecycle of the machine-written description.
///
/// `stale` is the interesting one: it means `notes_raw` or an upstream
/// influence changed after the last enhance. The UI offers a quiet re-enhance
/// rather than silently regenerating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptionState {
    /// Never enhanced.
    #[default]
    None,
    /// An enhance job is in flight.
    Enhancing,
    /// Machine-written and current.
    Fresh,
    /// Machine-written, then edited by hand.
    Edited,
    /// Upstream or notes changed since the last enhance.
    Stale,
}

/// A single description section. Prose sections are [`SectionValue::Text`];
/// `palette`, `signature` and `never` are [`SectionValue::List`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SectionValue {
    Text(String),
    List(Vec<String>),
}

impl SectionValue {
    pub fn value_kind(&self) -> SectionValueKind {
        match self {
            SectionValue::Text(_) => SectionValueKind::Text,
            SectionValue::List(_) => SectionValueKind::List,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            SectionValue::Text(t) => t.trim().is_empty(),
            SectionValue::List(items) => items.iter().all(|i| i.trim().is_empty()),
        }
    }
}

/// The structured, LLM-enhanced description. Ordered, because the section order
/// is the kind's declared order and that is what the editor renders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Description {
    #[serde(default)]
    pub sections: IndexMap<String, SectionValue>,
}

impl Description {
    /// Build from an ordered sequence of sections. Exists so a crate that
    /// assembles a description — the `wobu-llm` response validator — does not
    /// have to depend on `indexmap` just to fill in the one public field.
    pub fn from_sections(
        sections: impl IntoIterator<Item = (String, SectionValue)>,
    ) -> Description {
        Description { sections: sections.into_iter().collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.sections.values().all(|v| v.is_empty())
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.sections.get(key) {
            Some(SectionValue::Text(t)) => Some(t.as_str()),
            _ => None,
        }
    }

    pub fn list(&self, key: &str) -> Option<&[String]> {
        match self.sections.get(key) {
            Some(SectionValue::List(items)) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// The `never` section, which every kind declares and which becomes
    /// negative-prompt input.
    pub fn never(&self) -> &[String] {
        self.list("never").unwrap_or(&[])
    }

    /// Reorder to the kind's declared section order and drop sections that kind
    /// does not declare. Applied on read so a hand-edited or older file still
    /// renders predictably.
    pub fn normalised_for(&self, kind: NodeKind) -> Description {
        let declared = kind_def(kind).sections;
        let mut sections = IndexMap::new();
        for def in declared {
            if let Some(value) = self.sections.get(def.key) {
                sections.insert(def.key.to_string(), value.clone());
            }
        }
        Description { sections }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    pub name: String,
    /// The filename stem this node is stored under. Derived from `name`, but
    /// stored explicitly because renaming a node must not silently move its file
    /// out from under a collaborator.
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    /// Nesting *within* a kind: Region → City → District.
    #[serde(default)]
    pub parent_id: Option<Id>,
    /// The user's messy source notes. Never machine-written.
    #[serde(default)]
    pub notes_raw: String,
    #[serde(default)]
    pub description: Option<Description>,
    #[serde(default)]
    pub description_state: DescriptionState,
    #[serde(default)]
    pub attributes: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The image that represents this entity in a card or a tile. Nothing
    /// requires it to be linked as well — a cover is a choice about display,
    /// not about influence, and conflating the two would mean picking a
    /// thumbnail silently changed what gets sent to a backend.
    #[serde(default)]
    pub cover_asset_id: Option<Id>,
    #[serde(default)]
    pub links: Vec<Link>,
    /// Reference images attached to this entity, each with a role that decides
    /// where it is routed. See [`AssetRef`].
    #[serde(default)]
    pub asset_links: Vec<AssetRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Node {
    pub fn new(kind: NodeKind, name: impl Into<String>) -> Result<Node> {
        let name = name.into();
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::EmptyName);
        }
        let id = crate::new_id();
        // A name with nothing sluggable (all non-ASCII, say) still needs a file,
        // so fall back to the ULID rather than refusing to create the node.
        let slug = crate::slug::slugify(trimmed).unwrap_or_else(|_| id.to_string().to_lowercase());
        let now = Utc::now();
        Ok(Node {
            id,
            kind,
            name: trimmed.to_string(),
            slug,
            summary: String::new(),
            parent_id: None,
            notes_raw: String::new(),
            description: None,
            description_state: DescriptionState::None,
            attributes: serde_json::Map::new(),
            tags: Vec::new(),
            cover_asset_id: None,
            links: Vec::new(),
            asset_links: Vec::new(),
            created_at: now,
            updated_at: now,
        })
    }

    pub fn summary_row(&self) -> NodeSummary {
        NodeSummary {
            id: self.id,
            kind: self.kind,
            name: self.name.clone(),
            slug: self.slug.clone(),
            summary: self.summary.clone(),
            parent_id: self.parent_id,
            description_state: self.description_state,
        }
    }

    /// Flip a current description to `stale`. Called when notes or an upstream
    /// influence change. Deliberately a no-op for descriptions that were never
    /// written or are already being regenerated.
    pub fn mark_stale(&mut self) {
        if matches!(self.description_state, DescriptionState::Fresh | DescriptionState::Edited) {
            self.description_state = DescriptionState::Stale;
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Structural invariants that hold regardless of the rest of the world.
    /// Cross-node checks (cycles, singleton uniqueness) need the whole set and
    /// live in [`validate_parent`].
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::EmptyName);
        }
        if !crate::slug::is_valid_slug(&self.slug) {
            return Err(Error::UnslugifiableName(self.slug.clone()));
        }
        if self.parent_id == Some(self.id) {
            return Err(Error::SelfParent);
        }
        if self.parent_id.is_some() && !kind_def(self.kind).nests {
            return Err(Error::KindDoesNotNest { kind: self.kind.as_str() });
        }
        Ok(())
    }
}

/// Check a proposed parent against the rest of the world: same kind, nesting
/// allowed, and no cycle. `lookup` resolves an id to its (kind, parent_id).
pub fn validate_parent(
    child: &Node,
    parent_id: Option<Id>,
    lookup: &dyn Fn(Id) -> Option<(NodeKind, Option<Id>)>,
) -> Result<()> {
    let Some(parent_id) = parent_id else { return Ok(()) };

    if parent_id == child.id {
        return Err(Error::SelfParent);
    }
    if !kind_def(child.kind).nests {
        return Err(Error::KindDoesNotNest { kind: child.kind.as_str() });
    }

    let Some((parent_kind, _)) = lookup(parent_id) else {
        return Err(Error::CrossKindParent);
    };
    if parent_kind != child.kind {
        return Err(Error::CrossKindParent);
    }

    // Walk up from the proposed parent; meeting the child means this move would
    // detach a subtree into a ring, which the navigator would then fail to render.
    let mut cursor = Some(parent_id);
    let mut guard = 0;
    while let Some(id) = cursor {
        if id == child.id {
            return Err(Error::ParentCycle {
                child: child.name.clone(),
                parent: parent_id.to_string(),
            });
        }
        guard += 1;
        if guard > 10_000 {
            // The file on disk already contains a cycle; refuse rather than hang.
            return Err(Error::ParentCycle {
                child: child.name.clone(),
                parent: parent_id.to_string(),
            });
        }
        cursor = lookup(id).and_then(|(_, p)| p);
    }
    Ok(())
}

/// The lightweight row the navigator binds to. `node_list` returns these so the
/// tree can render without loading every node's notes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    pub id: Id,
    pub kind: NodeKind,
    pub name: String,
    pub slug: String,
    pub summary: String,
    pub parent_id: Option<Id>,
    pub description_state: DescriptionState,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: NodeKind, name: &str) -> Node {
        Node::new(kind, name).unwrap()
    }

    #[test]
    fn new_node_derives_a_slug_from_its_name() {
        let n = node(NodeKind::Character, "Kael Vantris");
        assert_eq!(n.slug, "kael-vantris");
        assert_eq!(n.description_state, DescriptionState::None);
        n.validate().unwrap();
    }

    #[test]
    fn unsluggable_names_fall_back_to_the_ulid() {
        let n = node(NodeKind::Character, "日本語");
        assert!(crate::slug::is_valid_slug(&n.slug), "{} should be usable", n.slug);
    }

    #[test]
    fn blank_names_are_rejected() {
        assert!(matches!(Node::new(NodeKind::Prop, "   "), Err(Error::EmptyName)));
    }

    #[test]
    fn mark_stale_only_touches_a_description_that_exists() {
        let mut n = node(NodeKind::Species, "Vashk");
        n.mark_stale();
        assert_eq!(n.description_state, DescriptionState::None, "nothing to invalidate");

        n.description_state = DescriptionState::Fresh;
        n.mark_stale();
        assert_eq!(n.description_state, DescriptionState::Stale);

        n.description_state = DescriptionState::Enhancing;
        n.mark_stale();
        assert_eq!(n.description_state, DescriptionState::Enhancing, "a running job wins");
    }

    #[test]
    fn non_nesting_kinds_reject_a_parent() {
        let mut n = node(NodeKind::Character, "Kael");
        n.parent_id = Some(crate::new_id());
        assert!(matches!(n.validate(), Err(Error::KindDoesNotNest { .. })));
    }

    #[test]
    fn parent_must_be_the_same_kind() {
        let child = node(NodeKind::Setting, "Cinder Bay");
        let culture_id = crate::new_id();
        let lookup = |id: Id| (id == culture_id).then_some((NodeKind::Culture, None));
        assert!(matches!(
            validate_parent(&child, Some(culture_id), &lookup),
            Err(Error::CrossKindParent)
        ));
    }

    #[test]
    fn reparenting_into_own_descendant_is_a_cycle() {
        // Ember Coast → Cinder Bay. Moving Ember Coast under Cinder Bay is a ring.
        let region = node(NodeKind::Setting, "Ember Coast");
        let city_id = crate::new_id();
        let region_id = region.id;
        let lookup =
            move |id: Id| (id == city_id).then_some((NodeKind::Setting, Some(region_id)));

        assert!(matches!(
            validate_parent(&region, Some(city_id), &lookup),
            Err(Error::ParentCycle { .. })
        ));
    }

    #[test]
    fn a_valid_reparent_is_accepted() {
        let city = node(NodeKind::Setting, "Cinder Bay");
        let region_id = crate::new_id();
        let lookup = |id: Id| (id == region_id).then_some((NodeKind::Setting, None));
        validate_parent(&city, Some(region_id), &lookup).unwrap();
    }

    #[test]
    fn description_normalises_to_the_kinds_declared_order() {
        let mut sections = IndexMap::new();
        // Deliberately out of order, plus a section characters do not declare.
        sections.insert("never".into(), SectionValue::List(vec!["Modern firearms".into()]));
        sections.insert("silhouette".into(), SectionValue::Text("Tall, narrow".into()));
        sections.insert("climate".into(), SectionValue::Text("does not belong".into()));

        let normalised = Description { sections }.normalised_for(NodeKind::Character);
        let keys: Vec<_> = normalised.sections.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["silhouette", "never"]);
    }

    #[test]
    fn section_values_serialise_as_a_tagged_union() {
        let text = serde_json::to_string(&SectionValue::Text("a".into())).unwrap();
        assert_eq!(text, r#"{"type":"text","value":"a"}"#);
        let list = serde_json::to_string(&SectionValue::List(vec!["a".into()])).unwrap();
        assert_eq!(list, r#"{"type":"list","value":["a"]}"#);
    }

    #[test]
    fn node_round_trips_through_json() {
        let mut n = node(NodeKind::Character, "Kael Vantris");
        n.notes_raw = "scarred, ex-guild".into();
        n.links.push(Link::new(crate::new_id(), crate::link::LinkRole::MemberOf));
        n.asset_links.push(AssetRef::new(crate::new_id(), crate::asset::AssetRole::Pose));
        n.cover_asset_id = Some(crate::new_id());
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(back, n);
    }
}
