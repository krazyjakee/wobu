# Agent access (MCP)

Wobu can let an AI assistant running on your computer — a coding assistant, a writing assistant —
read the world you have open, and, if you say so a second time, change it. Wobu can also use tools
you run yourself. All of it is switched off until you turn it on, and it lives in Settings → Agent
access (MCP).

> **Off by default, in both directions** Until you tick something here, Wobu is not listening for
> anything and is not running anything. Nothing on this page sends anything to anyone: both switches
> are about programs on this computer.

## Three separate switches

1. **Let an assistant on this computer read the open world.** Starts listening. Reading only.
2. **Let a connected assistant change this world.** Only appears once the first is on, and asks you
   to confirm, because it means another program can write files in your world folder.
3. **Let Wobu use tools you run.** The other way round entirely: Wobu doing the asking.

## Letting an assistant in

Wobu listens **only to this computer** — that cannot be changed, on purpose — on a port number you
can pick. Every request has to carry a password, and Settings gives you three things:

- **Show token**, since it is hidden until you ask.
- **Copy connection details**, which puts a ready-made setup on your clipboard.
- **New token**, which is also how you shut everything out: making a new one disconnects every
  assistant you had set up.

A few guards explain why something might be refused. Requests that look like they came from a web
page are turned away before the password is even checked, there is no way to stream data out, and
requests have a size limit. A website in your browser cannot reach Wobu — only a program running on
your own machine can.

### What an assistant can read

| Tool | |
| --- | --- |
| `world_overview` | The shape of the world |
| `list_nodes` | Everything in it, by sort |
| `get_node` | One page, with its notes and description |
| `search_nodes` | The same search **Jump to…** uses |
| `get_node_links` | What one page is joined to |
| `resolve_influence` | Everything that would feed a picture of it |
| `compile_prompt` | The prompt that would come out |
| `list_generations`, `get_generation` | Records of pictures you have made |

It can also fetch the world itself, the list of pages, and any single page.

### What it cannot do

There is **no deleting and no generating**. An assistant connected to Wobu cannot destroy anything
and cannot start a job, so nothing it does can spend your money. Even with writing switched on, all
it can do is make a page, update a page, and join two pages together — and it cannot write the
machine's description. An assistant contributing writing puts it in the notes column, where your own
words go, and Enhance stays something a person presses.

When writing is off, those tools are not offered at all, so it does not try and then fail.

### What has been going on

The panel keeps the recent requests — each marked `ok` or `refused`, with the name of the tool and,
for a refusal, why. It is the quickest way to find out that an assistant is asking for something you
have not allowed. Older ones are in the diagnostics log.

## Letting Wobu use your tools

Switch on **Let Wobu use MCP servers you run** and add them by name, command and arguments. Each one
is a program on this computer that Wobu will start as you, with exactly the arguments you gave —
nothing in between interpreting them. A newly added one starts switched off; you turn it on when you
are ready. **Check it works** starts it, asks what it can do, and tells you.

Those programs are stopped when you switch one off, edit it, remove it, or quit Wobu, and every
request has a time limit.

## Where these settings live

In a file called `mcp.json` in Wobu's own application folder, next to your other settings, readable
only by you because it holds the password. It belongs to this installation and is never inside a
world, so switching this on for yourself does not switch it on for everybody who opens your world.
Delete the file and everything here is off and the password is forgotten.

The full detail, including exactly when a request is refused, is in `docs/16-mcp.md` with the source
code.
