//! Markdown + YAML frontmatter is the source of truth.
//!
//! Frontmatter holds the structural facts (id, kind, links, attributes, tags);
//! the body holds `## Notes` and `## Description`. Keeping the prose as real
//! Markdown headings is what makes Obsidian work on a project folder as-is,
//! which is a stated goal in `docs/02-data-model.md`.

use std::path::Path;

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use wobu_core::{
    Description, DescriptionState, Id, Link, LinkRole, Node, NodeKind, SectionValue,
    SectionValueKind, kind_def,
};

use crate::error::{Error, Result};

const NOTES_HEADING: &str = "## Notes";
const DESCRIPTION_HEADING: &str = "## Description";

#[derive(Debug, Serialize, Deserialize)]
struct FmLink {
    to: Id,
    role: LinkRole,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    weight: f32,
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    enabled: bool,
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}
fn is_yes(v: &bool) -> bool {
    *v
}

/// The frontmatter block. Field names are deliberately short and stable —
/// people will hand-edit these in Obsidian.
#[derive(Debug, Serialize, Deserialize)]
struct Frontmatter {
    id: Id,
    kind: NodeKind,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cover: Option<Id>,
    #[serde(default)]
    description_state: DescriptionState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    links: Vec<FmLink>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    attributes: serde_json::Map<String, serde_json::Value>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub fn to_markdown(node: &Node) -> Result<String> {
    let fm = Frontmatter {
        id: node.id,
        kind: node.kind,
        name: node.name.clone(),
        summary: node.summary.clone(),
        parent: node.parent_id,
        tags: node.tags.clone(),
        cover: node.cover_asset_id,
        description_state: node.description_state,
        links: node
            .links
            .iter()
            .map(|l| FmLink { to: l.to_id, role: l.role, weight: l.weight, enabled: l.enabled })
            .collect(),
        attributes: node.attributes.clone(),
        created_at: node.created_at,
        updated_at: node.updated_at,
    };

    let yaml = serde_norway::to_string(&fm)?;

    let mut out = String::with_capacity(node.notes_raw.len() + 256);
    out.push('\n');
    out.push_str(NOTES_HEADING);
    out.push_str("\n\n");
    let notes = node.notes_raw.trim_end();
    if !notes.is_empty() {
        out.push_str(notes);
        out.push('\n');
    }

    if let Some(description) = &node.description
        && !description.is_empty()
    {
        out.push('\n');
        out.push_str(DESCRIPTION_HEADING);
        out.push('\n');
        for def in kind_def(node.kind).sections {
            let Some(value) = description.sections.get(def.key) else { continue };
            if value.is_empty() {
                continue;
            }
            out.push_str("\n### ");
            out.push_str(def.label);
            out.push_str("\n\n");
            match value {
                SectionValue::Text(text) => {
                    out.push_str(text.trim());
                    out.push('\n');
                }
                SectionValue::List(items) => {
                    for item in items.iter().filter(|i| !i.trim().is_empty()) {
                        out.push_str("- ");
                        out.push_str(item.trim());
                        out.push('\n');
                    }
                }
            }
        }
    }

    Ok(crate::frontmatter::join(&yaml, &out))
}

pub fn from_markdown(text: &str, path: &Path) -> Result<Node> {
    let crate::frontmatter::Split { yaml, body } = crate::frontmatter::split(path, text)?;

    let fm: Frontmatter = serde_norway::from_str(yaml).map_err(|e| Error::Malformed {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    let (notes, description_block) = split_body(body);
    let description = description_block
        .map(|block| parse_description(fm.kind, block))
        .filter(|d| !d.is_empty());

    let slug = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| fm.id.to_string().to_lowercase());

    Ok(Node {
        id: fm.id,
        kind: fm.kind,
        name: fm.name,
        slug,
        summary: fm.summary,
        parent_id: fm.parent,
        notes_raw: notes,
        description,
        description_state: fm.description_state,
        attributes: fm.attributes,
        tags: fm.tags,
        cover_asset_id: fm.cover,
        links: fm
            .links
            .into_iter()
            .map(|l| {
                Link { to_id: l.to, role: l.role, weight: l.weight, enabled: l.enabled }.clamped()
            })
            .collect(),
        created_at: fm.created_at,
        updated_at: fm.updated_at,
    })
}

/// Split the body into raw notes and the description block.
///
/// The description is machine-written and always last, so we split on the *last*
/// `## Description` heading — otherwise a user's own `##` headings inside their
/// notes would silently truncate them.
fn split_body(body: &str) -> (String, Option<&str>) {
    let notes_start = find_heading(body, NOTES_HEADING).map(|(_, end)| end).unwrap_or(0);

    match find_last_heading(body, DESCRIPTION_HEADING) {
        Some((start, end)) if start >= notes_start => {
            (body[notes_start..start].trim().to_string(), Some(&body[end..]))
        }
        _ => (body[notes_start..].trim().to_string(), None),
    }
}

/// Byte range `(start_of_line, start_of_next_line)` for the first line equal to
/// `heading`.
fn find_heading(body: &str, heading: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == heading {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

fn find_last_heading(body: &str, heading: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    let mut found = None;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == heading {
            found = Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    found
}

fn parse_description(kind: NodeKind, block: &str) -> Description {
    let mut sections: IndexMap<String, SectionValue> = IndexMap::new();
    let mut current: Option<&'static wobu_core::SectionDef> = None;
    let mut buffer: Vec<&str> = Vec::new();

    let flush = |def: Option<&'static wobu_core::SectionDef>,
                     buffer: &mut Vec<&str>,
                     sections: &mut IndexMap<String, SectionValue>| {
        let Some(def) = def else {
            buffer.clear();
            return;
        };
        let value = match def.value_kind {
            SectionValueKind::List => SectionValue::List(
                buffer
                    .iter()
                    .filter_map(|line| {
                        let t = line.trim();
                        t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).map(str::trim)
                    })
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            ),
            SectionValueKind::Text => SectionValue::Text(buffer.join("\n").trim().to_string()),
        };
        buffer.clear();
        if !value.is_empty() {
            sections.insert(def.key.to_string(), value);
        }
    };

    for line in block.lines() {
        if let Some(heading) = line.trim_end().strip_prefix("### ") {
            flush(current, &mut buffer, &mut sections);
            current = section_for_heading(kind, heading.trim());
        } else {
            buffer.push(line);
        }
    }
    flush(current, &mut buffer, &mut sections);

    // Re-order to the kind's declared order; a hand-edited file may list them
    // in any order at all.
    Description { sections }.normalised_for(kind)
}

/// Match a `### ` heading back to a declared section. Accepts either the label
/// we wrote (`Signature details`) or the raw key (`signature`), so a
/// hand-edited file still round-trips.
fn section_for_heading(kind: NodeKind, heading: &str) -> Option<&'static wobu_core::SectionDef> {
    let needle = heading.trim().to_ascii_lowercase();
    kind_def(kind).sections.iter().find(|def| {
        def.label.to_ascii_lowercase() == needle || def.key.to_ascii_lowercase() == needle
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("nodes/character/kael-vantris.md")
    }

    fn character() -> Node {
        let mut n = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
        n.summary = "Ex-guild enforcer".into();
        n.notes_raw = "scarred, ex-guild\nowes a debt".into();
        n.tags = vec!["exile".into()];
        n.links.push(Link::new(wobu_core::new_id(), LinkRole::MemberOf));

        let mut sections = IndexMap::new();
        sections.insert(
            "silhouette".into(),
            SectionValue::Text("Tall, narrow-shouldered, forward-canted stance.".into()),
        );
        sections.insert(
            "palette".into(),
            SectionValue::List(vec!["#2b2118".into(), "#c2703a".into()]),
        );
        sections.insert("never".into(), SectionValue::List(vec!["Modern firearms".into()]));
        n.description = Some(Description { sections });
        n.description_state = DescriptionState::Fresh;
        n
    }

    #[test]
    fn round_trips_a_full_node() {
        let original = character();
        let text = to_markdown(&original).unwrap();
        let parsed = from_markdown(&text, &path()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn round_trips_a_node_with_nothing_in_it() {
        let original = Node::new(NodeKind::Prop, "Ashglass Lantern").unwrap();
        let text = to_markdown(&original).unwrap();
        let parsed = from_markdown(&text, &PathBuf::from("nodes/prop/ashglass-lantern.md")).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn writes_readable_markdown() {
        let text = to_markdown(&character()).unwrap();
        assert!(text.starts_with("---\n"), "frontmatter fence first");
        assert!(text.contains("\n## Notes\n"));
        assert!(text.contains("\n### Silhouette\n"));
        assert!(text.contains("\n- #2b2118\n"), "lists render as markdown bullets");
        assert!(text.contains("kind: character"));
    }

    #[test]
    fn user_headings_inside_notes_are_not_treated_as_the_description() {
        // Someone writing structured notes in Obsidian will absolutely do this.
        let mut n = character();
        n.notes_raw = "## Backstory\n\nfled the guild\n\n## Wants\n\na way home".into();
        let text = to_markdown(&n).unwrap();
        let parsed = from_markdown(&text, &path()).unwrap();
        assert_eq!(parsed.notes_raw, n.notes_raw, "notes must survive intact");
        assert_eq!(parsed.description, n.description);
    }

    #[test]
    fn slug_comes_from_the_filename_not_the_name() {
        // Renaming a node must not silently move the file, so disk wins.
        let text = to_markdown(&character()).unwrap();
        let parsed = from_markdown(&text, &PathBuf::from("nodes/character/legacy-name.md")).unwrap();
        assert_eq!(parsed.slug, "legacy-name");
        assert_eq!(parsed.name, "Kael Vantris");
    }

    #[test]
    fn accepts_hand_edited_frontmatter_with_only_required_keys() {
        let text = "---\n\
                    id: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
                    kind: species\n\
                    name: Vashk\n\
                    created_at: 2026-07-31T14:22:11Z\n\
                    updated_at: 2026-07-31T14:22:11Z\n\
                    ---\n\n\
                    ## Notes\n\n\
                    ash-adapted\n";
        let node = from_markdown(text, &PathBuf::from("nodes/species/vashk.md")).unwrap();
        assert_eq!(node.name, "Vashk");
        assert_eq!(node.notes_raw, "ash-adapted");
        assert_eq!(node.description_state, DescriptionState::None);
        assert!(node.links.is_empty());
    }

    #[test]
    fn accepts_raw_section_keys_as_headings() {
        let text = "---\n\
                    id: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
                    kind: species\n\
                    name: Vashk\n\
                    created_at: 2026-07-31T14:22:11Z\n\
                    updated_at: 2026-07-31T14:22:11Z\n\
                    ---\n\n\
                    ## Notes\n\n\
                    x\n\n\
                    ## Description\n\n\
                    ### never\n\n\
                    - Symmetrical faces\n";
        let node = from_markdown(text, &PathBuf::from("nodes/species/vashk.md")).unwrap();
        assert_eq!(node.description.unwrap().never(), ["Symmetrical faces"]);
    }

    #[test]
    fn unknown_sections_are_dropped_rather_than_kept_as_junk() {
        let text = "---\n\
                    id: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
                    kind: species\n\
                    name: Vashk\n\
                    created_at: 2026-07-31T14:22:11Z\n\
                    updated_at: 2026-07-31T14:22:11Z\n\
                    ---\n\n\
                    ## Notes\n\n\
                    x\n\n\
                    ## Description\n\n\
                    ### Architecture\n\n\
                    species do not declare this\n\n\
                    ### Anatomy\n\n\
                    Four-jointed digitigrade legs.\n";
        let node = from_markdown(text, &PathBuf::from("nodes/species/vashk.md")).unwrap();
        let d = node.description.unwrap();
        assert_eq!(d.sections.len(), 1);
        assert_eq!(d.text("anatomy"), Some("Four-jointed digitigrade legs."));
    }

    #[test]
    fn tolerates_crlf_and_a_bom() {
        let text = "\u{feff}---\r\n\
                    id: 01ARZ3NDEKTSV4RRFFQ69G5FAV\r\n\
                    kind: species\r\n\
                    name: Vashk\r\n\
                    created_at: 2026-07-31T14:22:11Z\r\n\
                    updated_at: 2026-07-31T14:22:11Z\r\n\
                    ---\r\n\r\n\
                    ## Notes\r\n\r\n\
                    ash-adapted\r\n";
        let node = from_markdown(text, &PathBuf::from("nodes/species/vashk.md")).unwrap();
        assert_eq!(node.name, "Vashk");
    }

    #[test]
    fn a_file_without_frontmatter_is_an_error_not_a_silent_empty_node() {
        let err = from_markdown("just some prose\n", &path()).unwrap_err();
        assert!(matches!(err, Error::MissingFrontmatter(_)));
    }

    #[test]
    fn a_malformed_frontmatter_names_the_file() {
        let err = from_markdown("---\nkind: [unclosed\n---\n", &path()).unwrap_err();
        match err {
            Error::Malformed { path: p, .. } => assert_eq!(p, path()),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn hand_edited_link_weights_are_clamped_on_read() {
        let text = "---\n\
                    id: 01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
                    kind: character\n\
                    name: Kael\n\
                    links:\n\
                    \x20 - to: 01BX5ZZKBKACTAV9WEVGEMMVRZ\n\
                    \x20   role: member_of\n\
                    \x20   weight: 9.0\n\
                    created_at: 2026-07-31T14:22:11Z\n\
                    updated_at: 2026-07-31T14:22:11Z\n\
                    ---\n\n\
                    ## Notes\n";
        let node = from_markdown(text, &path()).unwrap();
        assert_eq!(node.links[0].weight, 1.0);
    }
}
