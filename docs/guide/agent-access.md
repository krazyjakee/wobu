# Agent access (MCP)

Wobu can let a coding or writing agent on your computer read the open world through the Model
Context Protocol — and, if you say so a second time, change it. It can also use MCP servers you run.
All of it is off until you switch it on, and it lives in Settings → Agent access (MCP).

> **Off by default, and off in two directions** Until you tick something here, Wobu listens on no
> port and runs no other program. Nothing on this pane sends anything to anyone: both switches are
> about this computer.

## Three separate opt-ins

1. **Let an agent on this computer read the open world.** Starts the server. Read-only.
2. **Let a connected agent change this world.** Appears only once the first is on, and asks for
   confirmation, because it means another program can write files in your project folder.
3. **Let Wobu use MCP servers you run.** The other direction entirely: Wobu as the client.

## The server

It listens on **127.0.0.1 only** — the address is not configurable, by design — on a port you can
change, and speaks JSON-RPC over HTTP POST at `/mcp`. Every request must carry a bearer token, and
Settings gives you three things to work with:

- **Show token**, since it is masked until you ask.
- **Copy connection details**, which puts a ready-made client configuration on the clipboard.
- **New token**, which is also how you revoke: rotating it disconnects every agent you configured.

A few guards are worth knowing because they explain refusals. Any request carrying an `Origin`
header is refused before the token is even checked, no CORS header is ever sent, there is no `GET`
and no event stream, and bodies are capped. A web page in a browser therefore cannot reach it, only
a local process can.

### What an agent can read

| Tool | |
| --- | --- |
| `world_overview` | The shape of the project |
| `list_nodes` | Every entity, by kind |
| `get_node` | One entity, with its notes and description |
| `search_nodes` | The same full-text search the palette uses |
| `get_node_links` | An entity's influence edges |
| `resolve_influence` | The resolved stack for an entity |
| `compile_prompt` | What that stack compiles to |
| `list_generations`, `get_generation` | Generation receipts |

There are also three resources — the project, the node list, and each node by id.

### What it cannot do

There is **no delete tool and no generate tool**. An agent connected to Wobu cannot destroy anything
and cannot start a job, so nothing it does can spend your money. Even with writes enabled, the three
write tools are create a node, update a node, and link two nodes — and `update_node` cannot write
the machine's description. An agent contributing prose writes into the raw notes, where your own
words go, and Enhance remains something a person presses.

When writes are off, the write tools are not offered to the agent at all, so it does not try and
then fail.

### Activity

The pane keeps the recent calls — each one `ok` or `refused`, with the tool name and, for a refusal,
why. It is the fastest way to find out that an agent is asking for something you have not enabled.
Older calls are in the diagnostics log.

## The client

Switch on **Let Wobu use MCP servers you run** and add servers by name, command and arguments. Each
one is a program on this computer that Wobu will start as you, over stdio, with the arguments you
give it — no shell in between. A newly added server is switched off; you turn it on when you are
ready. **Check it works** starts it, asks what tools it has, and reports back.

Child processes are killed when a server is disabled, edited, removed, or when Wobu exits, and every
request has a deadline.

## Where the settings live

In `mcp.json` in Wobu's application data directory, beside the machine settings, readable only by
you because it holds the token. It is per installation and never inside a project, so switching this
on does not switch it on for everyone who opens your world. Deleting the file turns everything off
and forgets the token.

The full protocol detail, including the exact refusal rules, is in the repository's `docs/16-mcp.md`.
