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
                // is given, `link_nodes` adds an edge, `create_scene` adds a
                // file and `draft_dialogue` only ever fills a slot that was
                // empty; all four are recoverable by hand, and none removes a
                // node, an asset, a scene or a line. There is no MCP tool that
                // deletes anything, on purpose.
                "destructiveHint": false,
                "idempotentHint": !self.write
                    || !matches!(self.name, "create_node" | "create_scene"),
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

fn scene_id_only() -> Value {
    object(
        json!({ "sceneId": { "type": "string", "description": "The scene's ULID." } }),
        &["sceneId"],
    )
}

fn list_scenes_schema() -> Value {
    object(
        json!({
            "query": {
                "type": "string",
                "description":
                    "Optional free text over scene names, beat intent and dialogue. When set, \
                     each row also carries the lines that matched.",
            },
            "act": { "type": "string", "description": "Act EntityId, from narrative_overview." },
            "arc": { "type": "string", "description": "Arc EntityId." },
            "quest": { "type": "string", "description": "Quest EntityId. A scene may be in several." },
            "tag": { "type": "string", "description": "Tag EntityId." },
            "participant": { "type": "string", "description": "Character node id, as list_nodes reports it." },
            "review": { "type": "string", "enum": ["draft", "approved"] },
            "freshness": { "type": "string", "enum": ["current", "out_of_date"] },
            "missingText": {
                "type": "boolean",
                "description": "Only scenes with a dialogue slot that has no words in it yet.",
            },
            "offset": { "type": "integer", "minimum": 0 },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25 },
        }),
        &[],
    )
}

fn narrative_search_schema() -> Value {
    object(
        json!({
            "query": {
                "type": "string",
                "description": "Free text. Matches scene names, beat intent and dialogue lines.",
            },
            "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 25 },
        }),
        &["query"],
    )
}

fn text_asset_schema() -> Value {
    object(
        json!({ "assetId": { "type": "string", "description": "The text asset's ULID." } }),
        &["assetId"],
    )
}

fn narrative_diagnostics_schema() -> Value {
    object(
        json!({
            "sceneId": {
                "type": "string",
                "description": "Optional. Omit to check every scene in the project.",
            }
        }),
        &[],
    )
}

fn create_scene_schema() -> Value {
    object(json!({ "name": { "type": "string", "minLength": 1 } }), &["name"])
}

fn draft_dialogue_schema() -> Value {
    object(
        json!({
            "sceneId": { "type": "string" },
            "slotId": {
                "type": "string",
                "description":
                    "The dialogue slot to fill, from get_scene. It must have no variants yet \
                     and must not be locked.",
            },
            "body": {
                "type": "string",
                "minLength": 1,
                "description": "The line, as it would be spoken. Not a condition and not a command.",
            },
        }),
        &["sceneId", "slotId", "body"],
    )
}

fn add_dialogue_slot_schema() -> Value {
    object(
        json!({
            "sceneId": { "type": "string" },
            "beatId": {
                "type": "string",
                "description": "The beat to add the line to, from get_scene.",
            },
            "speaker": {
                "type": "string",
                "description":
                    "\"narrator\" for unattributed narration, or the id of a character who is \
                     already a participant in this scene. The player cannot be given lines here, \
                     and a character who is not in the cast is refused: adding somebody to a \
                     scene is the writer's decision.",
            },
            "position": {
                "type": "string",
                "enum": ["start", "end"],
                "description":
                    "Where in the beat the new line goes. Defaults to the end. Use \"start\" for \
                     establishing narration before a beat's first spoken line.",
            },
            "body": {
                "type": "string",
                "minLength": 1,
                "description": "The line, as it would be spoken or read. Not a condition and not a command.",
            },
        }),
        &["sceneId", "beatId", "speaker", "body"],
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
        name: "narrative_overview",
        title: "Narrative overview",
        description: "Whether this project has an authored story, and its shape: scene and \
             supporting-text counts, how many dialogue slots are still empty, and the acts, arcs, \
             tags and quests a scene can be filed under, with the ids the other narrative tools \
             filter by.",
        write: false,
        schema: empty,
    },
    Tool {
        name: "list_scenes",
        title: "List scenes",
        description: "Scene rows from the same library the writer uses: name, act, arc, quests, \
             tags, cast, beat and slot counts, and how much of the text is generated, edited, \
             locked, awaiting review or out of date. Filterable and paged. With a query, each row \
             also carries the lines that matched.",
        write: false,
        schema: list_scenes_schema,
    },
    Tool {
        name: "search_narrative",
        title: "Search the story",
        description: "Full-text search over scene names, beat intent and dialogue, including \
             unapproved drafts. Each hit names the scene, beat, slot and variant it is in, so a \
             line can be read in place with get_scene.",
        write: false,
        schema: narrative_search_schema,
    },
    Tool {
        name: "get_scene",
        title: "Read a scene",
        description: "One scene document in full: participants, entry condition, beats in author \
             order, each beat's intents, must-convey and must-not-reveal notes, dialogue slots and \
             their variants with revision and review state, choices, outcomes and tombstones.",
        write: false,
        schema: scene_id_only,
    },
    Tool {
        name: "narrative_state",
        title: "Read the declared state",
        description: "Every state variable the story may branch on — name, type, owner, default and \
             range. Read this before writing anything that mentions a condition: a name that is not \
             declared here cannot appear in one.",
        write: false,
        schema: empty,
    },
    Tool {
        name: "narrative_world",
        title: "Read the story canon",
        description: "Facts, what each character believes and how they came to believe it, \
             relationships, events, quests and the facts withheld until a condition — with the \
             diagnostics they currently raise. A belief here may be false; that is the point of \
             recording it separately from the fact.",
        write: false,
        schema: empty,
    },
    Tool {
        name: "list_text_assets",
        title: "List supporting text",
        description: "Barks, letters, item descriptions and the rest: the supporting text assets in \
             this project, as summaries with their kind.",
        write: false,
        schema: empty,
    },
    Tool {
        name: "get_text_asset",
        title: "Read one supporting text asset",
        description: "One supporting text asset in full: its trigger, cast, repeat policy and every \
             authored entry with its condition and lines.",
        write: false,
        schema: text_asset_schema,
    },
    Tool {
        name: "narrative_diagnostics",
        title: "Check the story",
        description: "What is wrong with one scene, or with every scene: destinations that point \
             nowhere, conditions over undeclared variables, references to entities that are not in \
             the project. Reading them changes nothing and repairs nothing.",
        write: false,
        schema: narrative_diagnostics_schema,
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
    Tool {
        name: "create_scene",
        title: "Create a scene",
        description: "Add an empty scene to the story. Writes a new YAML file into the user's \
             project folder; nothing that exists is touched.",
        write: true,
        schema: create_scene_schema,
    },
    Tool {
        name: "draft_dialogue",
        title: "Draft a line into an empty slot",
        description: "Put words into a dialogue slot that has none yet, as an unreviewed draft \
             marked as coming from outside Wobu. Refused for a slot that already has a wording and \
             for a locked slot: nothing here replaces a line anybody wrote. It writes one line and \
             cannot add a beat, a choice, an outcome or a condition, so it cannot change where the \
             story goes.",
        write: true,
        schema: draft_dialogue_schema,
    },
    Tool {
        name: "add_dialogue_slot",
        title: "Add a line to a beat",
        description: "Append or prepend a new dialogue line to an existing beat, as an unreviewed \
             draft marked as coming from outside Wobu. For narration a scene is missing — an \
             establishing line before a beat's first spoken line — and for an extra line where a \
             beat has only one. The speaker is the narrator or a character already in the scene's \
             cast. It adds a line and nothing else: no beat, choice, outcome, effect, condition or \
             destination, so it cannot change where the story goes.",
        write: true,
        schema: add_dialogue_slot_schema,
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
    fn the_write_set_is_exactly_the_tools_the_disclosure_names() {
        // The Settings pane tells the user which tools the write opt-in turns
        // on. If this list grows, that sentence is wrong until somebody updates
        // it — which is what this test is for.
        let writes: Vec<_> = CATALOGUE.iter().filter(|t| t.write).map(|t| t.name).collect();
        assert_eq!(
            writes,
            [
                "create_node",
                "update_node",
                "link_nodes",
                "create_scene",
                "draft_dialogue",
                "add_dialogue_slot"
            ]
        );
    }

    #[test]
    fn the_privacy_policy_can_say_how_many_tools_there_are_because_this_says_so() {
        // `docs/legal/privacy-policy.md` §3.5 states the size of the read
        // surface and the shape of the write one. A count in a legal document
        // that nothing checks is a count that goes stale on the next commit.
        let reads = CATALOGUE.iter().filter(|tool| !tool.write).count();
        let writes = CATALOGUE.len() - reads;
        assert_eq!((reads, writes), (18, 6), "update the privacy policy and the guide with these");
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
