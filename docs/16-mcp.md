# 16 — Agent Access (MCP)

Wobu speaks the Model Context Protocol in both directions, and both are off until somebody
turns them on.

- **Server** — an agent running on the same computer can read the open world: nodes, links,
  the resolved influence stack, the compiled prompt, and the receipt for every generation.
  Writing is a second, separate decision.
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

**Write (only after the second opt-in)**

| Tool | Does |
| --- | --- |
| `create_node` | Adds an entity, writing a new Markdown file |
| `update_node` | Changes name, summary, source notes, tags or attributes |
| `link_nodes` | Adds an influence edge, changing what future prompts contain |

There is no tool that deletes anything, and no tool that starts a generation. An agent
connected to Wobu cannot spend money.

`update_node` cannot write the *generated* description. That field carries a freshness state
and a stamp of what the last Enhance read, and a write that set the prose without them would
leave a node claiming to be freshly enhanced from notes it has never seen. An agent that wants
to contribute prose writes `notes_raw`, which is the field for exactly that.

Three resources are offered alongside the tools: `wobu://project`, `wobu://nodes`, and
`wobu://node/{id}`.

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
- No tool that deletes, and no tool that spends.

## Where the code is

| Path | What |
| --- | --- |
| `src-tauri/crates/wobu-mcp/` | Protocol, the loopback listener, the stdio client, the tool catalogue |
| `src-tauri/src/mcp.rs` | Settings, the listener handle, the `World` implementation, the audit log |
| `src/components/McpSection.tsx` | The pane, its disclosure and the write confirmation |
| `src/lib/mcp.ts` | The typed command wrappers |

`wobu-mcp` knows nothing about `Project`, `NodeKind` or influence layers. Everything an agent
can reach is one method on its `World` trait, that trait fits on a screen, and a new capability
cannot appear without a line being added to it in review.
