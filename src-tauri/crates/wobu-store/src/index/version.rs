//! The hashes that say whether a node's meaning changed.
//!
//! Deliberately not a hash of the file: reformatting, reordering links or a
//! trailing newline must not invalidate an enhancement. What is hashed is the
//! text and the edges, normalized.

use wobu_core::Node;

/// The version of a node **as an influence source** — what a description
/// downstream of it was enhanced from.
///
/// Two things go in. **The description**, because the enhance context is built
/// from the resolved stack's descriptions rather than their raw notes
/// (`docs/04-influence-engine.md`); rewriting a species' notes cannot have
/// changed a word of a character enhanced under it, so it must not invalidate
/// one. **The edges** — `parent_id` and the enabled links — because they decide
/// which sources a stack reaches at all: a culture that gains a `located_in`
/// puts a whole place chain in front of every character in it, and that is a
/// change to their input even though no description moved.
///
/// The edges are also what make a stamp self-sufficient. A source can only
/// enter a stack through an edge on a node already in that stack, so a stamp
/// that watches its recorded sources' edges cannot miss one appearing — and
/// staleness needs no walk to answer.
///
/// Left out: `name`, `summary`, link `weight`, `updated_at`. Weight is a
/// compile-time knob that never reaches the enhance context, and the rest are
/// labels. Marking a hundred descriptions stale because somebody fixed a typo
/// in a species name is the noise that teaches people to ignore the dot.
pub fn source_version(node: &Node) -> String {
    let description = node
        .description
        .as_ref()
        .map(|d| serde_json::to_string(d).unwrap_or_default())
        .unwrap_or_default();
    version(&[&description, &edges(node)])
}

/// The version of a node **as an enhance subject** — its own half of the
/// context.
///
/// Its notes and attributes, which are what the model is asked to elaborate,
/// plus its edges, which decide its stack. Its own description is deliberately
/// absent: that is the *output*, and including it would make a description
/// stale the instant it was written, and make every hand-edit register as
/// staleness rather than as the resolution it is.
pub fn subject_version(node: &Node) -> String {
    let attributes = serde_json::to_string(&node.attributes).unwrap_or_default();
    version(&[&node.notes_raw, &attributes, &edges(node)])
}

/// A node's edges, canonically. Sorted rather than left in file order: shuffling
/// the `links:` block in Obsidian changes nothing the enhance context would see,
/// and hashing file order would report that reshuffle as a change to every
/// description downstream.
pub(super) fn edges(node: &Node) -> String {
    let mut out: Vec<String> = node
        .links
        .iter()
        .filter(|l| l.enabled)
        .map(|l| format!("{}:{}", l.to_id, l.role.as_str()))
        .collect();
    out.sort();
    if let Some(parent) = node.parent_id {
        out.insert(0, format!("{parent}:parent"));
    }
    out.join(",")
}

/// BLAKE3 over length-prefixed parts.
///
/// Length-prefixed rather than delimited because a delimiter can appear inside
/// somebody's notes, and `["a", "bc"]` hashing the same as `["ab", "c"]` is a
/// change this would then never see.
///
/// Truncated to 64 bits, which goes into frontmatter a person reads in
/// Obsidian. This answers "did these bytes change", not "could an adversary
/// forge these bytes"; a full hash would be four lines of noise per source in
/// every enhanced file, and people abandon formats that look like that.
pub(super) fn version(parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().to_hex()[..16].to_string()
}

pub(super) fn description_text(node: &Node) -> String {
    let Some(description) = &node.description else { return String::new() };
    let mut out = String::new();
    for value in description.sections.values() {
        match value {
            wobu_core::SectionValue::Text(t) => {
                out.push_str(t);
                out.push('\n');
            }
            wobu_core::SectionValue::List(items) => {
                for item in items {
                    out.push_str(item);
                    out.push('\n');
                }
            }
        }
    }
    out
}
