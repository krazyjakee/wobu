# Keyboard, settings and files

Shortcuts, every settings section, the on-disk layout, and a glossary.

## Keyboard

Every key Wobu listens for is declared in one registry, and **all of them are yours to change**.
Settings → Keyboard lists them, lets you press a new chord onto any row, shows the shipped default
beside anything you have customised, and offers **Reset every shortcut**. The same list, read-only,
is one keystroke away from anywhere — the in-app [shortcuts reference](wobu:shortcuts).

So treat the table below as *what this build ships with*, not as fact about your installation. It is
also why tooltips throughout the app print the chord rather than the guide doing it: the tooltip is
read from the registry and follows a rebind.

`Mod` is Command on macOS and Control everywhere else. Bindings are stored once, not per platform;
only the way they are drawn changes.

| Default | Command |
| --- | --- |
| `Mod+K` | Command palette |
| `Mod+/` | Keyboard shortcuts |
| `Mod+F` | Filter the navigator |
| `Mod+Shift+L` | Library |
| `Mod+\` | Forge, and back |
| `Mod+Shift+A` | Assets |
| `Mod+,` | Settings |
| `[` | Toggle the navigator |
| `]` | Toggle the inspector |
| `Mod+Shift+C` | Collapse or expand everything |
| `Mod+1` … `Mod+5` | Notes, References, Concepts, 3D, Relations |
| `Mod+N` | New entity |
| `Mod+Z`, `Mod+Shift+Z` | Undo, redo |
| `Mod+E` | Enhance the open entity |
| `Mod+Enter` | Generate |

Three things are fixed rather than configurable, because a build where they were not would be
broken rather than flexible: **Escape** dismisses whatever is on top, **Tab** and **Shift+Tab** move
within a dialog and stay inside it, and the **arrow keys and Enter** choose in a palette or a menu.

Most shortcuts stand down while you are typing, so `[` in your notes is just a bracket. The
exceptions are deliberate and named in the reference — the command palette is reachable
mid-sentence, because it is how you leave where you are.

Undo and redo are the interesting case: inside a text box, `Mod+Z` belongs to the box, not to the
world. Stealing it would rewind a whole save every time somebody tried to take back a word.

> **Two commands, one chord** Binding a chord that is already taken is allowed, and Wobu tells you
> exactly what happened rather than letting you discover it by pressing the key: the first command
> in the registry wins, the others do nothing, and both the bindings editor and the shortcuts
> reference name the winner and offer to restore the default. Chords the system already claims —
> copy, paste, quit, reload — are allowed too, with a warning that the system's action will happen
> as well.

## Settings

| Section | What is in it |
| --- | --- |
| Providers and models | What the project uses, and the keys on this computer. See [Providers and keys](providers.md). |
| Legal | The terms and the privacy policy, read from the files beside the application, and what you accepted. |
| Introduction | Runs the first-run introduction again. It never re-asks for your agreement. |
| Agent access (MCP) | The opt-in MCP server and client. See [Agent access](agent-access.md). |
| Storage | Where the local index lives, its size and node count, and **Rebuild index**. |
| Editor | The autosave delay. Raise it on a slow share. |
| Appearance | Theme — *Match system*, *Light* or *Dark* — and interface scale. |
| Keyboard | Every binding, rebindable, with conflicts named. |
| Static world wiki | Exports a browsable, self-contained site. Only appears with a project open. |
| Diagnostics | Log file path and size, log level, reveal, and an in-app tail you can read and copy. |
| About | Wobu version, project schema version, index schema version. |
| Licences | Wobu's own MIT licence and the generated third-party notices for this build. |

> **Rebuilding the index is safe** The index holds no canonical data — it is derived from the
> Markdown. Delete it or rebuild it and nothing is lost; the next open just takes longer. If search
> results look wrong or a file seems stuck, rebuild before you investigate anything else.

### The static wiki export

**Export static wiki…** writes a plain HTML site — no JavaScript, no build step — with a page per
node carrying its notes, description, attributes, references, concepts and relations, an index
grouped by kind, and an influence graph drawn as SVG. It only reads the project, so it works on a
read-only folder, and it refuses to write into a folder that already exists or that sits inside the
project.

## On-disk layout

A project is a self-contained folder, not a database file. Put it on a NAS, a USB stick, or in git.

```
Ashfall.wobu/
├── project.json                  id, name, schema version, provider selection, spend ceiling
├── nodes/
│   ├── style-guide/              the Art Style singleton
│   ├── world-bible/              the World Canon singleton
│   ├── species/vashk.md          YAML frontmatter + notes + description
│   ├── culture/ember-guild.md
│   ├── setting/cinder-bay.md
│   ├── character/kael-vantris.md
│   └── creature/  prop/  environment/  vehicle/
├── assets/
│   ├── originals/a3/a3f9…c1.png  content-addressed, sharded by the first 2 hex characters
│   ├── thumbs/a3/a3f9…c1.webp
│   ├── loras/7d/7d42…9e.safetensors
│   └── meshes/7b/7b21…04.glb
├── generations/2026-07/<ulid>.json
└── .wobu/
    ├── sessions/<session-id>.json  heartbeats — who else has this open
    ├── spend/                      the spend ledger
    └── tmp/                        staging for atomic writes
```

**Markdown is the source of truth.** Frontmatter holds the id, kind, parent, links, attributes and
tags; the body holds `## Notes` and `## Description`. You get git history on your world, external
editing — Obsidian works on this folder as-is — trivial backup, and no migration dread.

### Rules that keep the folder portable

| | |
| --- | --- |
| No absolute paths | The same share is one path on your machine and another on somebody else's. Everything internal is relative. |
| Content-addressed assets | Two people importing the same reference produce the same file. Deduplication is free and asset writes never conflict. |
| Conservative filenames | Lowercase ASCII slugs, nothing Windows or a case-insensitive SMB share would reject, and shallow nesting to stay under path-length limits. |
| No symlinks | They do not survive SMB, zip, or most sync clients. |
| Write-once generations | ULID-named, never mutated. Append-only means no conflict surface. |
| No secrets, ever | Keys are in your OS keychain. `project.json` records only which provider a project prefers. |

### What is deliberately *not* in the folder

The search index, your keybindings, your favourites, your theme, the ComfyUI address, the MCP token
and the record that you accepted the terms all live on this machine rather than in the project —
because each of them is a fact about you or your computer, not about the world. Push one of them
into the folder and it becomes everyone's.

The index in particular lives in local application data, keyed by the project's id rather than its
path, so it survives the share being remounted somewhere else. SQLite over SMB or NFS relies on
advisory locks that are unreliable to broken, and the documented failure mode is corruption rather
than an error. Copy a project folder to a machine that has never seen it and nothing is lost; the
first open rebuilds.

## Troubleshooting

| Symptom | Try |
| --- | --- |
| Search misses something you know is there | Settings → Storage → Rebuild index. |
| A node shows as corrupt | Reveal it and open the file in a text editor — usually broken frontmatter from a hand edit or a sync client. Wobu will not write over it until it parses. |
| Everything went read-only | Check share permissions. Wobu decides this once, when the project opens. |
| Banner says the share is offline | Wobu is retrying with backoff. The interface stays readable from the index; held writes resume when the mount returns. |
| Enhance or Generate is disabled | No key on this machine for the selected provider, or the folder is read-only. Settings → Providers and models. |
| Generate errors on a working Gemini key | Image models need billing enabled; text is free. See [Providers and keys](providers.md). |
| Mesh auth fails for no reason | Check your system clock — signatures expire after a few minutes of skew. |
| A job failed and you have lost the message | The notification centre in the status bar keeps every failure, with what it cost. |
| Reference images seem to do nothing | Check the role, and check the backend's capabilities — the influence stack reports every reference it did not send. |
| Editing feels laggy on a NAS | Raise the autosave delay in Settings → Editor. |
| Something needs reporting | Settings → Diagnostics. Raise the log level, reproduce, then read the tail in-app before you send it — keys are redacted, but you should still see what you are handing over. |

## Glossary

| | |
| --- | --- |
| Node, entity | Any record in the world. Species, character, prop, and the two singletons all share one record type. |
| Kind | What a node is. Selects its icon, colour, influence layer, description sections and default relations. |
| Singleton | Art Style and World Canon — one of each per project, pinned above the rule. |
| Link | An influence edge between nodes, with a role and a weight. |
| Layer | One rung of the resolved stack: style, world, ancestry, culture, place, subject, shot. |
| Fragment | One piece of text or one image contributed by a layer, with a weight and a routing target. |
| Influence stack | The resolved, ordered set of layers feeding the current generation. |
| Compiled prompt | The final prompt string, attributable to the layer that produced each span. |
| Influence snapshot | The exact resolved stack saved with a generation, making it replayable later. |
| Role | What a reference image is *for* — silhouette, palette, material, mood, pose, costume, full reference. |
| Stale | A description whose notes or upstream layers changed since it was enhanced. |
| Preset | An output recipe — which sections to boost, framing, aspect, image count. |
| Pinning | Promoting a generated image to a reference on its node, so it feeds the next generation. |
| Take | One attempt at a single turnaround view. Every take is kept. |
| Ticket | An access credential for peer-to-peer project sync, created from the project menu and accepted from the launcher. Tickets must be shared privately. |

## Design documentation

This guide describes the product. The reasoning behind it — and the constraints that shaped each
decision — lives in the design documents in the repository: [Vision](../01-vision.md),
[Data model](../02-data-model.md), [UI layout](../03-ui-layout.md),
[Influence engine](../04-influence-engine.md), [Architecture](../05-architecture.md),
[File shares](../07-file-shares.md), [Providers](../08-providers.md), [MCP](../16-mcp.md),
[Roadmap](../09-roadmap.md).
