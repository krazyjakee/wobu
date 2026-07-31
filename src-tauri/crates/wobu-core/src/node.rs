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
/// Four of these five are facts about what somebody *did*: never enhanced, a
/// job is running, the machine wrote this, a person rewrote it. They belong in
/// the Markdown and they survive there, because the folder is the only copy of
/// them.
///
/// [`Stale`](DescriptionState::Stale) is not that kind of fact. "Is this
/// description still current" is a question about the rest of the world — the
/// Style Guide's description, a culture's links — and its answer changes
/// without anybody touching this node's file. Storing it would mean an edit to
/// the Style Guide had to rewrite every node it reaches: a hundred guarded
/// writes over a share, a hundred chances to park a conflict sibling, and a
/// hundred files whose `updated_at` moved for a change the user did not make to
/// them. It would also destroy information, because the enum holds one value at
/// a time: flipping `edited` to `stale` forgets that a person wrote those words
/// by hand, and the next enhance would overwrite them without knowing to ask.
///
/// So Wobu never writes this variant. Staleness is *derived*, by comparing a
/// node's [`EnhanceStamp`] against the world as it currently stands, and it is
/// derived in the index — the disposable half — so that recomputing it costs
/// nothing and writes nothing.
///
/// The variant stays because a *file* can still say `stale`: one written by an
/// older Wobu, or by a person in Obsidian marking something for redoing. That
/// is honoured on read. It is only never written.
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
    /// Upstream or notes changed since the last enhance. Read, never written —
    /// see the type's own documentation.
    Stale,
}

/// One upstream source as it stood when a description was enhanced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    pub node: Id,
    pub version: String,
}

/// What the last enhance read, recorded so that staleness can be computed at
/// all.
///
/// Nothing else in a Markdown file says what the model was shown, so without
/// this there is no way to tell a description that is current from one whose
/// sources have moved on — and the app would have to either regenerate
/// constantly or never.
///
/// A "version" here is a **content hash**, and that choice is the load-bearing
/// one. `updated_at` is the obvious alternative and it is wrong three ways: a
/// hand-edit in Obsidian rewrites a description without touching it, so the
/// change that matters most is the one it cannot see; two machines on one share
/// disagree about the clock, so a current description can look older than the
/// world it came from; and [`Node::touch`] moves it on every save, so
/// re-saving a node unchanged would mark everything downstream stale.
/// Filesystem mtime is worse still — it survives neither a copy, nor a sync
/// client, nor a `git clone`, and the first thing a fresh checkout would do is
/// declare the whole project stale. A hash is a fact about the content rather
/// than about the machine the content is sitting on, so it survives all of
/// them.
///
/// What goes *into* each hash is decided in `wobu_store::index`, next to the
/// column that caches it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnhanceStamp {
    /// The subject's own contribution to its enhance context — its notes, its
    /// attributes, and the edges that decide which sources it reaches.
    pub subject: String,
    /// Every source the stack resolved to, in stack order. The subject is not
    /// among them: its own description is the *output* of the enhance, so
    /// stamping it would make every node stale the instant it was written.
    #[serde(default)]
    pub sources: Vec<SourceStamp>,
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
    /// What the last enhance read. `None` for a description nobody has enhanced
    /// through Wobu — including one typed straight into the Markdown — which is
    /// read as "cannot be shown to be out of date" rather than as stale. See
    /// [`EnhanceStamp`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enhanced_from: Option<EnhanceStamp>,
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
            enhanced_from: None,
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

    /// Whether a re-enhance would throw away words a person wrote.
    ///
    /// The one question the enhance path must ask before it writes, and the
    /// reason `edited` is never folded into `stale`: once the state has been
    /// overwritten there is nothing left to ask.
    pub fn description_is_hand_written(&self) -> bool {
        self.description_state == DescriptionState::Edited
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
    fn a_new_node_carries_no_enhance_stamp() {
        // A node nobody has enhanced must not look enhanced-and-current, or the
        // first thing that reads a stamp would compare against an empty one and
        // call every fresh project stale.
        let n = node(NodeKind::Species, "Vashk");
        assert_eq!(n.enhanced_from, None);
        assert!(!n.description_is_hand_written());
    }

    #[test]
    fn a_hand_written_description_announces_itself() {
        // The single check standing between a re-enhance and somebody's prose.
        let mut n = node(NodeKind::Species, "Vashk");
        n.description_state = DescriptionState::Fresh;
        assert!(!n.description_is_hand_written());
        n.description_state = DescriptionState::Edited;
        assert!(n.description_is_hand_written());
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
        n.enhanced_from = Some(EnhanceStamp {
            subject: "0123456789abcdef".into(),
            sources: vec![SourceStamp {
                node: crate::new_id(),
                version: "fedcba9876543210".into(),
            }],
        });
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn an_unstamped_node_does_not_serialise_an_empty_stamp() {
        // The `doc` column and the bridge both carry this. An `enhancedFrom` of
        // nulls would parse back as a stamp with no sources, which reads as
        // "enhanced against nothing" — the one shape that is indistinguishable
        // from a node whose whole stack was deleted.
        let json = serde_json::to_value(node(NodeKind::Prop, "Lantern")).unwrap();
        assert!(json.get("enhancedFrom").is_none(), "{json}");
    }
}
