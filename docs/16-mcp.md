# 16 — Agent Access (MCP)

Wobu speaks the Model Context Protocol in both directions, and both are off until somebody
turns them on.

- **Server** — an agent running on the same computer can read the open project: the world
  model — nodes, links, the resolved influence stack, the compiled prompt, and the receipt
  for every generation — and the authored story on top of it: scenes, beats, dialogue,
  declared state and canon. Writing is a second, separate decision.
- **Client** — Wobu can use MCP servers the user already runs, so their own tools are
  available while they work.

Everything below lives in **Settings → Agent access (MCP)**. Nothing on that pane sends
anything off the machine: the server listens on loopback only, and the client talks to
programs on this computer over their standard input and output.

## The three decisions

They are deliberately three, not one, and each is independent.

| Decision | What it does | Default |
| --- | --- | --- |
| Let an agent read the open world | Binds `127.0.0.1:<port>` and answers MCP over it | Off |
| Let a connected agent change this world | Adds the three write tools | Off |
| Let Wobu use MCP servers you run | Allows configured servers to be launched | Off |

Turning any of them on or off takes effect immediately. There is no restart, and "off" is not
a flag consulted at request time — the socket is closed and the child processes are killed.

## What the server exposes

The pane lists these by name, generated from the same catalogue the protocol advertises, so
the disclosure cannot drift from the implementation.

**Read (available whenever the server is on)**

| Tool | Answers |
| --- | --- |
| `world_overview` | Project name, folder, read-only status, node counts by kind |
| `list_nodes` | Every entity as a summary, optionally filtered to one kind |
| `get_node` | One entity in full, with links, attributes and attached reference images |
| `search_nodes` | Full-text search over names, summaries and notes |
| `get_node_links` | The influence edges into and out of one node |
| `resolve_influence` | The layered stack for a subject, and how each layer was reached |
| `compile_prompt` | The positive and negative prompt a generation would send |
| `list_generations` | Generation receipts for one node — model, seed, cost, outcome |
| `get_generation` | One receipt in full |
| `narrative_overview` | Whether there is a story, its scene and asset counts, and the acts, arcs, tags and quests to filter by |
| `list_scenes` | Scene rows from the library: classification, cast, beat and slot counts, text coverage |
| `search_narrative` | Full-text search over scene names, beat intent and dialogue, drafts included |
| `get_scene` | One scene document in full — beats, slots, variants, choices, outcomes, tombstones |
| `narrative_state` | The declared variables: name, type, owner, default and range |
| `narrative_world` | Facts, beliefs, relationships, events, quests, restrictions, with their diagnostics |
| `list_text_assets` | Supporting text assets, as summaries |
| `get_text_asset` | One asset in full, with its entries |
| `narrative_diagnostics` | What is wrong with one scene, or with every scene |

**Write (only after the second opt-in)**

| Tool | Does |
| --- | --- |
| `create_node` | Adds an entity, writing a new Markdown file |
| `update_node` | Changes name, summary, source notes, tags or attributes |
| `link_nodes` | Adds an influence edge, changing what future prompts contain |
| `create_scene` | Adds an empty scene, writing a new YAML file |
| `draft_dialogue` | Puts one wording into a dialogue slot that has none |

There is no tool that deletes anything, and no tool that starts a generation. An agent
connected to Wobu cannot spend money.

`update_node` cannot write the *generated* description. That field carries a freshness state
and a stamp of what the last Enhance read, and a write that set the prose without them would
leave a node claiming to be freshly enhanced from notes it has never seen. An agent that wants
to contribute prose writes `notes_raw`, which is the field for exactly that.

The narrative writes are the same shape of decision, and there are three things to say about
them.

**Both are additive.** `create_scene` writes a new file and touches nothing that exists.
`draft_dialogue` is refused for a slot that already has a wording and for a locked slot, so
nothing an agent does over MCP replaces a line anybody wrote. There is no whole-document scene
write at all: a scene save is guarded by the stamp the reader held, which is what stops two
writers clobbering each other on a shared folder, and an agent posting one stateless request
at a time holds no such thing. Reading the file inside the write and saving over whatever is
there would turn a detected conflict into a silent overwrite of somebody's afternoon.

**Neither can author a branch.** `draft_dialogue` writes one unconditional wording into a slot
a person already made. It cannot add a beat, a choice, an outcome, an effect or a condition.
That is the line [the narrative system](17-narrative-system.md) draws around generation, held
here for the same reason: prose is inert, and where the story goes is the writer's statement.

**The wording is recorded as `Imported`, never as `Human` and never as `Generated`.**
Provenance is half of what a revision hashes and the review queue reads it to decide what it is
looking at. `Human` would tell a reviewer a person typed a line nobody in the project has read;
`Generated` would claim a receipt that does not exist, because there is no job, no model and no
fingerprint behind an MCP call. The line arrives as an unreviewed draft with the cautious
generation policy, which is to say in the review queue and not in a build.

Five resources are offered alongside the tools: `wobu://project`, `wobu://nodes`,
`wobu://node/{id}`, `wobu://scenes`, and `wobu://scene/{id}`.

## Connecting an agent

With the server on, the pane shows the address and a token. **Copy connection details** puts
this on the clipboard:

```json
{
  "mcpServers": {
    "wobu": {
      "type": "http",
      "url": "http://127.0.0.1:9628/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

The port is configurable and defaults to `9628`. The address is not: see below.

**New token** replaces the credential, which immediately stops every agent configured with the
old one. That is the revocation path.

## What guards the port

Loopback is not a trust boundary on a desktop — every process on the machine can reach it, and
so can a page in a browser. So:

- **The address is loopback and is not configurable.** The port is a setting; `127.0.0.1` is
  written in one place in `wobu-mcp`'s server module and there is no setting, file or flag that
  makes it bind anything else. Reaching Wobu from another machine is an SSH port forward, which
  is a decision made outside Wobu by somebody who knows they are making it.
- **Every request needs the bearer token**, compared in constant time.
- **Any request carrying an `Origin` header is refused** before the token is looked at, and no
  CORS header is ever sent. A real MCP client is a program and does not send `Origin`; a web
  page always does. This is what closes DNS rebinding, and refusing before authentication means
  a page cannot tell a right token from a wrong one.
- **There is no `GET`, and no event stream.** One JSON-RPC POST, one answer.
- **Bodies are capped** at one megabyte, measured rather than trusted.

## Using servers you run

Each configured server is a program on this computer that Wobu starts as you, with the
arguments you give it, over stdio. There is no smaller version of that — it is what an MCP
stdio server is — so:

- A server is added **switched off**. Adding is not running.
- Nothing is launched unless both the master switch and that server's own switch are on.
- The command is invoked directly. Nothing goes through a shell, so a semicolon in a field is a
  character in an argument rather than a second command.
- Children are killed when the server is disabled, edited, removed, or when Wobu exits.
- Every request has a deadline, so a wedged server does not become a wedged Wobu.

Environment overrides for a server can be added by hand to `mcp.json` (below). Their values are
never sent to the interface — that is the likeliest place for one of your own API keys — so the
pane says only that a server has them.

## Where the settings live

`mcp.json` in Wobu's application data directory, beside `settings.json`, mode `0600` on Unix
because it holds the token. Per installation and never inside a project: a port on this machine
and a command on this machine's `PATH` are not things a collaborator at the other end of a
share could use.

Deleting the file turns everything off and forgets the token.

## Seeing what happened

Every tool call — successful or refused — is timestamped, listed in the pane, written to the
diagnostics log, and emitted as an event so the list updates live. A refused write appears too,
because "is something poking at my world" is a question about attempts, not just successes.

The activity list holds the last fifty calls; the diagnostics log holds the rest.

## What is deliberately not implemented

- No SSE or streaming transport, no sampling, no prompt registry, no server-initiated requests.
  The subset here is what a coding or writing agent actually uses.
- No per-call confirmation dialog for writes. The gate is the opt-in plus the audit trail; a
  modal raised from a background HTTP request that an agent may be making while nobody is at the
  keyboard is a worse guarantee than it looks.
- No tool that deletes, and no tool that spends. In particular nothing here starts a prose
  generation job, a variant build or an export.
- No whole scene document write, and no narrative tool that takes a stamp. See above: the
  precondition that makes a guarded save safe is one an agent cannot hold.

## Where the code is

| Path | What |
| --- | --- |
| `src-tauri/crates/wobu-mcp/` | Protocol, the loopback listener, the stdio client, the tool catalogue |
| `src-tauri/src/mcp.rs` | Settings, the listener handle, the `World` implementation, the audit log |
| `src-tauri/src/mcp/narrative.rs` | The `Narrative` implementation: scenes, canon, and the two additive writes |
| `src/components/McpSection.tsx` | The pane, its disclosure and the write confirmation |
| `src/lib/mcp.ts` | The typed command wrappers |

`wobu-mcp` knows nothing about `Project`, `NodeKind`, influence layers or beats. Everything an
agent can reach is one method on its `World` trait or on its `Narrative` trait, each fits on a
screen, and a new capability cannot appear without a line being added to one of them in review.

They are two traits rather than one because the two bodies of source are two — Markdown nodes
with influence edges, and scene documents with declared state — with different rules about what
may be written to each. One trait long enough to hold both is one nobody reads, which is the
failure the guarantee above exists to prevent.
