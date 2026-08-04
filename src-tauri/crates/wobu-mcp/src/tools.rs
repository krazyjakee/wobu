//! The catalogue: every tool an agent can be offered, and which of them write.
//!
//! This file is the disclosure. The sentence the Settings pane puts in front of
//! the user before they turn anything on is generated from [`catalogue`], so
//! there is no way to add a tool here and leave the UI describing the old set —
//! a test in `dispatch` pins the split, and the frontend renders the same names
//! and descriptions the protocol advertises.
//!
//! `description` is written for the model rather than for the user, because
//! that is what it is for; it is also what the user reads, so it says what the
//! tool touches rather than how it is implemented.

use serde_json::{Value, json};

/// One tool, as advertised and as gated.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Whether calling this changes the user's project. The whole of the second
    /// opt-in hangs off this one boolean.
    pub write: bool,
    schema: fn() -> Value,
}

impl Tool {
    /// The `tools/list` entry.
    ///
    /// `annotations` are hints rather than a security boundary — an agent is
    /// free to ignore them, which is precisely why the enforcement is in
    /// `dispatch` and not here. They are still worth sending: a client that
    /// asks the human before a destructive call can only do that if it is told
    /// which calls those are.
    pub fn describe(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": (self.schema)(),
            "annotations": {
                "title": self.title,
                "readOnlyHint": !self.write,
                // Nothing here deletes. `update_node` overwrites the fields it
                // is given and `link_nodes` adds an edge; both are recoverable
                // by hand, and neither removes a node or an asset. There is no
                // MCP tool that deletes anything, on purpose.
                "destructiveHint": false,
                "idempotentHint": !self.write || self.name != "create_node",
                "openWorldHint": false,
            },
        })
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn empty() -> Value {
    object(json!({}), &[])
}

fn id_only() -> Value {
    object(json!({ "id": { "type": "string", "description": "The node's ULID." } }), &["id"])
}

fn list_nodes_schema() -> Value {
    object(
        json!({
            "kind": {
                "type": "string",
                "description":
                    "Optional kind filter, e.g. character, species, culture, setting, \
                     creature, prop, environment, vehicle, style_guide, world_bible.",
            }
        }),
        &[],
    )
}

fn search_schema() -> Value {
    object(
        json!({
            "query": { "type": "string", "description": "Free text. Matches names, summaries and notes." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20 },
        }),
        &["query"],
    )
}

fn subject_schema() -> Value {
    object(
        json!({
            "subjectId": { "type": "string", "description": "The ULID of the node to resolve for." },
            "preset": {
                "type": "string",
                "description":
                    "Optional output preset id. Defaults to the subject kind's own preset.",
            },
        }),
        &["subjectId"],
    )
}

fn generations_schema() -> Value {
    object(
        json!({
            "nodeId": { "type": "string" },
            "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20 },
        }),
        &["nodeId"],
    )
}

fn generation_schema() -> Value {
    object(json!({ "generationId": { "type": "string" } }), &["generationId"])
}

fn create_node_schema() -> Value {
    object(
        json!({
            "kind": { "type": "string", "description": "One of the kinds list_nodes reports." },
            "name": { "type": "string", "minLength": 1 },
            "parentId": {
                "type": "string",
                "description": "Optional parent of the same kind, for kinds that nest.",
            },
        }),
        &["kind", "name"],
    )
}

fn update_node_schema() -> Value {
    object(
        json!({
            "id": { "type": "string" },
            "patch": {
                "type": "object",
                "description":
                    "Only the fields present are changed. The generated description is not \
                     writable here; contribute prose through notesRaw instead.",
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "summary": { "type": "string" },
                    "notesRaw": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "attributes": { "type": "object" },
                },
                "additionalProperties": false,
            },
        }),
        &["id", "patch"],
    )
}

fn link_schema() -> Value {
    object(
        json!({
            "nodeId": { "type": "string", "description": "The influenced node." },
            "toId": { "type": "string", "description": "The influencing node." },
            "role": {
                "type": "string",
                "enum": ["species_of", "member_of", "located_in", "styled_by", "related_to"],
            },
            "weight": { "type": "number", "minimum": 0, "maximum": 1 },
        }),
        &["nodeId", "toId", "role"],
    )
}

/// Everything, in the order an agent should meet it.
///
/// Reads first and writes last is not cosmetic: `tools/list` truncates in some
/// clients, and the read tools are the ones that are always there.
static CATALOGUE: &[Tool] = &[
    Tool {
        name: "world_overview",
        title: "World overview",
        description: "Name, folder, read-only status and node counts by kind for the Wobu project that \
             is currently open. Start here: every other tool needs ids from this world, and \
             this is the call that says whether one is open at all.",
        write: false,
        schema: empty,
    },
    Tool {
        name: "list_nodes",
        title: "List nodes",
        description: "Every entity in the open world as a summary — id, kind, name, parent, one-line \
             summary and tags. Optionally narrowed to a single kind.",
        write: false,
        schema: list_nodes_schema,
    },
    Tool {
        name: "get_node",
        title: "Read a node",
        description: "One entity in full: notes, generated description and its freshness, attributes, \
             tags, influence links and attached reference images.",
        write: false,
        schema: id_only,
    },
    Tool {
        name: "search_nodes",
        title: "Search the world",
        description: "Full-text search across names, summaries and notes, answered from the local \
             index. Returns node summaries, not bare ids.",
        write: false,
        schema: search_schema,
    },
    Tool {
        name: "get_node_links",
        title: "Read a node's influence edges",
        description: "The explicit influence edges into and out of one node, with role and weight. \
             Parent nesting is on the node itself rather than here.",
        write: false,
        schema: id_only,
    },
    Tool {
        name: "resolve_influence",
        title: "Resolve the influence stack",
        description: "The layered stack Wobu would resolve for a subject — style guide, world bible, \
             ancestry, culture, place, subject — with which node reached each layer, how it \
             was reached, and at what weight.",
        write: false,
        schema: subject_schema,
    },
    Tool {
        name: "compile_prompt",
        title: "Compile the prompt",
        description: "The positive and negative prompt text a generation for this subject would \
             actually send, plus the fragments it is assembled from and anything the budget \
             dropped. Compiles only; nothing is generated and nothing is spent.",
        write: false,
        schema: subject_schema,
    },
    Tool {
        name: "list_generations",
        title: "List generation receipts",
        description: "The recorded generations for one node: provider, model, seed, prompt, cost and \
             outcome. These are receipts for work already done — reading them spends nothing.",
        write: false,
        schema: generations_schema,
    },
    Tool {
        name: "get_generation",
        title: "Read one generation receipt",
        description: "One generation record in full, including its resolved settings and cost.",
        write: false,
        schema: generation_schema,
    },
    Tool {
        name: "create_node",
        title: "Create a node",
        description: "Add a new entity to the open world. Writes a Markdown file into the user's \
             project folder.",
        write: true,
        schema: create_node_schema,
    },
    Tool {
        name: "update_node",
        title: "Update a node",
        description: "Change a node's name, summary, source notes, tags or attributes. Only the fields \
             present in the patch are touched. Writes to the user's project folder.",
        write: true,
        schema: update_node_schema,
    },
    Tool {
        name: "link_nodes",
        title: "Link two nodes",
        description: "Add an explicit influence edge, which changes what future prompts for the \
             influenced node will contain. Writes to the user's project folder.",
        write: true,
        schema: link_schema,
    },
];

/// The whole catalogue, reads and writes alike.
pub fn catalogue() -> &'static [Tool] {
    CATALOGUE
}

/// What is advertised right now.
///
/// With writes off the write tools are not merely refused, they are invisible.
/// An agent that asks what it can do is entitled to a truthful answer, and
/// "here are three tools that will always fail" is not one.
pub fn advertised(allow_writes: bool) -> impl Iterator<Item = &'static Tool> {
    CATALOGUE.iter().filter(move |tool| allow_writes || !tool.write)
}

pub fn find(name: &str) -> Option<&'static Tool> {
    CATALOGUE.iter().find(|tool| tool.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_in_the_catalogue_is_named_twice() {
        let mut names: Vec<_> = CATALOGUE.iter().map(|tool| tool.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two tools share a name");
    }

    #[test]
    fn the_write_set_is_exactly_the_three_tools_the_disclosure_names() {
        // The Settings pane tells the user which tools the write opt-in turns
        // on. If this list grows, that sentence is wrong until somebody updates
        // it — which is what this test is for.
        let writes: Vec<_> = CATALOGUE.iter().filter(|t| t.write).map(|t| t.name).collect();
        assert_eq!(writes, ["create_node", "update_node", "link_nodes"]);
    }

    #[test]
    fn no_tool_deletes_anything() {
        // Not a naming convention — an actual property of the surface. Reading
        // is reversible and adding is recoverable; removing a node an agent
        // decided was redundant is neither, and there is no confirmation step
        // in a background HTTP request that could make it so.
        for tool in CATALOGUE {
            assert!(!tool.name.contains("delete"), "{} deletes", tool.name);
            assert!(!tool.name.contains("remove"), "{} removes", tool.name);
            assert_eq!(tool.describe()["annotations"]["destructiveHint"], false);
        }
    }

    #[test]
    fn with_writes_off_only_read_tools_are_advertised() {
        let read_only: Vec<_> = advertised(false).map(|t| t.name).collect();
        assert!(!read_only.is_empty());
        assert!(read_only.iter().all(|name| !find(name).unwrap().write));
        assert_eq!(advertised(true).count(), CATALOGUE.len());
    }

    #[test]
    fn every_schema_is_a_closed_object_so_a_typo_is_reported_rather_than_ignored() {
        for tool in CATALOGUE {
            let schema = (tool.schema)();
            assert_eq!(schema["type"], "object", "{}", tool.name);
            assert_eq!(schema["additionalProperties"], false, "{}", tool.name);
        }
    }

    #[test]
    fn read_tools_are_annotated_read_only() {
        for tool in CATALOGUE {
            assert_eq!(
                tool.describe()["annotations"]["readOnlyHint"],
                !tool.write,
                "{}",
                tool.name
            );
        }
    }
}
