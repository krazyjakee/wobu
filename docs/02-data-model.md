# 02 — Data Model

## The one big idea: everything is a Node

Species, characters, settings, props, environments, cultures, vehicles — and even the
project's Style Guide and World Bible — are all the **same record type**. They differ only in
`kind`, which selects an attribute schema, an icon, and a set of default relations.

This matters for the UI: there is exactly one editor shell to build, and users learn it once.
It also means adding "Factions" or "Technologies" later is a config change, not a feature.

```
Project
├── Style Guide      (kind: style_guide,  singleton, pinned)
├── World Bible      (kind: world_bible,  singleton, pinned)
└── Nodes            (any kind, freely nestable within kind)
```

## Records

### Node

| Field | Type | Notes |
| --- | --- | --- |
| `id` | ulid | |
| `kind` | enum | see kinds below |
| `name` | string | |
| `summary` | string | one-line, shown in tree tooltips and influence cards |
| `parent_id` | ulid? | nesting **within** a kind: Region → City → District |
| `notes_raw` | markdown | the user's messy source notes. Never machine-written. |
| `description` | json | structured, LLM-enhanced. See below. |
| `description_state` | enum | `none · enhancing · fresh · edited · stale` |
| `attributes` | json | kind-specific facts (scale, era, biome, material…); controls come from `KindDef.attributes` |
| `tags` | string[] | |
| `cover_asset_id` | ulid? | |
| `locked_seed` | u64? | shared entity identity seed; explicit re-rolls do not overwrite it |
| `created_at` / `updated_at` | ts | |

`description_state = stale` when `notes_raw` or an upstream influence changed after the last
enhance. The UI shows a quiet "re-enhance" affordance rather than silently regenerating.

### Structured description

Enhance produces sections, not a blob. Sections are kind-aware:

```json
{
  "silhouette":  "Tall, narrow-shouldered, forward-canted stance …",
  "anatomy":     "Four-jointed digitigrade legs; vestigial second pair of arms …",
  "materials":   "Ash-glazed ceramic plate over oiled leather …",
  "palette":     ["#2b2118", "#c2703a", "#7fa5a3"],
  "signature":   ["Ember-lit throat vents", "Guild signet fused to the collarbone"],
  "never":       ["Modern firearms", "Symmetrical faces"]
}
```

`never` becomes negative-prompt input. `palette` can drive a colour-conditioning pass.

### Link — the influence edge

| Field | Type | Notes |
| --- | --- | --- |
| `from_id` | ulid | the entity being described |
| `to_id` | ulid | the influence source |
| `role` | enum | `species_of · member_of · located_in · styled_by · related_to` |
| `weight` | 0.0–1.0 | default 1.0 |
| `enabled` | bool | mutable per-generation without editing the world |

Links are what the Influence Engine walks. `parent_id` (same-kind nesting) is treated as an
implicit link of weight 1.0.

### Asset & AssetLink

An **Asset** is a file on disk (`reference · generated · upload`) with dimensions, a content
hash, and a thumbnail. An **AssetLink** attaches it to a node with a **role**:

`silhouette · palette · material · mood · pose · costume · full_ref`

The role is what makes image context routable — a `palette` reference goes to colour
conditioning, a `pose` reference goes to a structure adapter, a `mood` reference may only
ever be shown to the human.

### Generation

Records `node_id`, the user's extra prompt, the **compiled prompt**, the negative prompt, the
model + params + seed, output asset ids, and an `influence_snapshot` — the exact resolved
stack, weights and all. That snapshot is what makes a result reproducible six months later
after the world has moved on.

The project-wide History view reads these append-only receipts from the disposable index; the
same receipt remains visible under a node's Concepts tab. Opening one shows its stored prompts,
request parameters and every snapshot fragment. Replay constructs the provider request directly
from those immutable fields and reference asset bytes, without resolving today's nodes or presets.
The replay receipt points back with `params.replayOf`; its current-price reservation is recorded
separately from the source receipt's original estimate. A missing snapshot reference is a hard
error, never an invitation to substitute a current node link.

A mesh receipt uses the typed `params.meshOutput` object rather than putting a GLB in the image
`outputAssetIds` list: `{ assetId, turnaroundGenerationIds }`. The source ids name the immutable
eight-view receipts that actually fed the mesh job. If an older or externally written receipt
does not carry the complete list, the 3D tab says the source sheet was not recorded; it never
guesses from whichever Turnaround happens to be newest now.

## Node kinds (v1)

| Kind | Nests | Typical influences |
| --- | --- | --- |
| `style_guide` | – | (root of every stack) |
| `world_bible` | – | style |
| `species` | yes (sub-species) | world |
| `culture` | yes | world, species |
| `setting` | yes (Region → City → District) | world, culture |
| `character` | no | species, culture, setting |
| `creature` | no | species, setting |
| `prop` | yes (sets) | culture, setting |
| `environment` | no | setting, culture |
| `vehicle` | no | culture, setting |

Adding a kind = adding a row to a registry: label, icon, colour, attribute schema, default
link roles, default output presets.

## On-disk format

A project is a **self-contained folder**, not a database file. Everything a project *is* —
notes, structure, images, generation history — lives inside that one directory, so it can be
put on a NAS, an SMB/NFS share, Dropbox, or a USB stick and opened by anyone who can see the
path. Nothing about a project is stored in a global application database.

```
Ashfall.wobu/
├── project.json                  id, name, schema version, providers, shared spend ceiling
├── nodes/
│   ├── species/vashk.md          YAML frontmatter + notes + description
│   ├── setting/cinder-bay.md
│   └── character/kael-vantris.md
├── assets/
│   ├── originals/a3/a3f9…c1.png  content-addressed, sharded by first 2 hex chars
│   ├── thumbs/a3/a3f9…c1.webp
│   ├── loras/7d/7d42…9e.safetensors  trained weights, content-addressed
│   └── meshes/7b/7b21…04.glb     concept 3D output
├── generations/2026-07/<ulid>.json
└── .wobu/
    ├── sessions/<session-id>.json  heartbeat locks — who else has this open
    ├── spend/reservations/*.json   paid batches admitted but not fully receipted
    └── tmp/                        staging for atomic writes (same filesystem)
```

**Markdown files are the source of truth.** Frontmatter holds `id`, `kind`, links, attributes
and tags; the body holds `## Notes` and `## Description`. This gives us git history on the
world, external editing (Obsidian works on this folder as-is), trivial backup, and no
migration dread.

### Rules that make the folder portable

These are constraints on the writer, and each one exists because something breaks otherwise:

- **No absolute paths, anywhere.** The same share is `/Volumes/art/Ashfall.wobu` on one
  machine and `Z:\art\Ashfall.wobu` on another. All internal references are relative and
  stored with `/` separators, converted on read.
- **Assets and trained LoRA weights are content-addressed** by BLAKE3 hash, sharded two levels
  deep. Two people
  importing the same reference produce the same file — so asset writes can never conflict,
  and dedup is free. The extension comes from the detected content type, never from the
  imported filename, or the same bytes dropped in as `ref.png` and `ref.PNG` would land at
  two paths and the property would be gone.
- **An asset's id is derived from its hash**, not minted. Nothing on disk records an asset
  id — the filename *is* the hash — so a minted one would be reissued whenever the index
  was rebuilt, and every `asset_id` already sitting in somebody's frontmatter would dangle.
  Deriving it also extends the conflict-free property from the bytes to the records that
  point at them. The cost: asset ids do not sort by creation time, because the bits a ULID
  normally spends on a timestamp hold hash instead. `created_at` is the field for that.
- **Filenames are lowercase ASCII slugs**, restricted to what Windows and case-insensitive
  SMB shares tolerate: no `< > : " | ? *`, no trailing dots or spaces, and no reserved names
  (`CON`, `PRN`, `AUX`, `NUL`, `COM1`…). Nesting stays shallow to keep total path length
  under Windows' 260-character default.
- **No symlinks.** They don't survive SMB, zip, or most sync clients.
- **Generation records are write-once**, named by ULID, never mutated. Append-only means no
  conflict surface.
- **Deleting a node promotes its children to the deleted node's parent.** Only the selected
  Markdown file is removed. Each child keeps its stable `nodes/<kind>/<slug>.md` path and has its
  `parent` frontmatter rewritten; links pointing at the deleted id are removed. This avoids both
  silently deleting a subtree and leaving dead influence edges behind.
- **Secrets are never written to the project folder.** API keys live in the OS keychain;
  `project.json` records only *which* provider and model a project prefers. This matters
  enormously now that folders are shared — see [08 — Providers & BYOK](08-providers.md).

### Where the index lives — and why it is not in the folder

Search, backlinks and graph traversal need an index, and SQLite is the obvious choice. But
**the SQLite file must not live inside the project folder**, because project folders are
explicitly meant to sit on network shares:

- SQLite's locking relies on POSIX advisory locks, which are unreliable-to-broken over SMB
  and NFS. The documented failure mode is database corruption, not an error message.
- WAL mode requires shared memory and **does not work at all** on network filesystems.
- Sync clients (Dropbox, OneDrive) will happily copy a half-written database mid-transaction.

So the index lives in local application data, keyed by the project's ULID rather than its
path, so it survives the share being remounted somewhere else:

```
~/.local/share/wobu/index/<project-ulid>.sqlite     (Linux; equivalents on macOS/Windows)
```

This does not violate the self-contained rule — the index holds **no** canonical data. It is a
cache of what's already in the Markdown, rebuilt from the folder whenever the schema version
or a content hash mismatches. Delete it, or copy the project folder to a machine that has
never seen it, and nothing is lost; the first open just takes a little longer.

Concurrency, conflict handling and the network-share performance story are in
[07 — Projects on file shares](07-file-shares.md).

### Importing a style or subtree

“Import style/subtree” reads another `.wobu` folder through a disposable local index. A selected
root brings only its `parent_id` descendants: explicit links between selected nodes are remapped to
fresh destination ids, while links outside the selection are reported in preview and omitted. A
selected root is detached from any unselected source parent.

Ordinary entities receive fresh ids and collision-free slugs. Importing a singleton such as the Art
Style replaces the destination singleton's authored content but preserves its destination id, slug
and `created_at`; existing destination links to that singleton therefore remain valid, and imported
descendants point to the preserved id. Source enhancement stamps are cleared and imported
descriptions are marked edited, because the destination did not run the enhancement that produced
them. Provider selections, secrets, generation history and spend records never transfer.

Every cover and reference blob is read and hash-checked before destination node publication. Missing
or mismatched blobs block apply; valid blobs keep their content-derived ids and deduplicate against
bytes already present. Asset roles, weights and muted state are retained, while thumbnails remain
derived and are regenerated lazily. A selected node's pinned LoRA frontmatter and immutable
`assets/loras/<prefix>/<hash>.safetensors` blob transfer together after safetensors, size, path and
content-hash checks. Transfer does not copy provider settings or assume the destination ComfyUI has
installed the provider filename; generation re-probes that machine and reports an explicit
downgrade when it cannot apply the project-owned weight.

The destination is also fully preflighted before its first node write. Guarded writes still matter on
a shared folder: if another author wins a race after preflight, the command returns an explicit
partial report containing applied and pending node ids plus any conflict sibling paths. It never
claims the transfer was atomic; already copied content-addressed blobs are safe reusable orphans.
