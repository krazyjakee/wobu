# 03 — UI Layout

## Shape of the app

One primary screen — **the Workspace** — with three vertical regions plus a mode rail. Users
live here 95% of the time. Everything else is a mode swap in the centre or a modal.

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  wobu   [ Ashfall ▾ ]   Species / Vashk / Kael Vantris        ⌘K search      ⚙︎     │ title bar
├────┬────────────────────┬──────────────────────────────┬───────────────────────────┤
│ ▤  │ ⌕ filter…          │  KAEL VANTRIS      ⋯  Enhance│  INFLUENCE STACK          │
│ ✦  │                    │  character · Vashk · Ember G.│  ┌──────────────────────┐ │
│ ⬚  │ ★ Art Style        │ ─────────────────────────────│  │ ● Art Style          │ │
│    │ ★ World Canon      │  Notes │ Refs │ Concepts │ 3D│  │   Ashfall House  1.0 ▸│ │
│    │                    │ ─────────────────────────────│  ├──────────────────────┤ │
│ ⚙  │ ▾ Species          │  ┌───────────┬──────────────┐│  │ ● World Canon    0.8 ▸│ │
│    │    Vashk           │  │ RAW NOTES │ DESCRIPTION  ││  ├──────────────────────┤ │
│    │    Sunborn         │  │           │ Silhouette   ││  │ ● Species             │ │
│    │ ▾ Cultures         │  │ scarred   │ Anatomy      ││  │   Vashk          1.0 ▸│ │
│    │    Ember Guild     │  │ ex-guild  │ Materials    ││  ├──────────────────────┤ │
│    │ ▾ Settings         │  │ …         │ Palette      ││  │ ● Culture             │ │
│    │  ▾ Ember Coast     │  │           │ Signature    ││  │   Ember Guild    0.6 ▸│ │
│    │     Cinder Bay     │  │           │ Never        ││  ├──────────────────────┤ │
│    │ ▾ Characters       │  └───────────┴──────────────┘│  │ ● Place / Subject …  │ │
│    │    Kael Vantris ◀  │                              │  ├──────────────────────┤ │
│    │ ▾ Props            │                              │  │ COMPILED PROMPT      │ │
│    │    Ashglass Lantern│                              │  │ ▓▓▓▓ tinted by layer │ │
│    │                    │                              │  ├──────────────────────┤ │
│    │ + New              │                              │  │ Preset ▾  Aspect ▾   │ │
│    │                    │                              │  │   ✦ Generate  ×4     │ │
├────┴────────────────────┴──────────────────────────────┴───────────────────────────┤
│ ComfyUI connected · flux-dev · queue 0        claude-opus-4-8 · 12k ctx      ⏱ 4.2s │ status
└────────────────────────────────────────────────────────────────────────────────────┘
   52px          272px                  fluid                       352px
```

## Regions

### Mode rail (52px)
Icon-only, always visible. `Library` (the tree above) · `Forge` (full-width generation +
result grid) · `Assets` (all images, filterable) · `Settings`. Keeps the top bar clean and
makes mode switching muscle memory.

### Navigator (272px, resizable)
Filter box at top, and under it one line giving the size of the world — `812 entities`, or
`14 of 812 shown` while the filter is narrowing — with **Collapse all** beside it. Two pinned
singletons — **Art Style** and **World Canon** — sit above the rule, because they are the
roots of every influence stack and should never be hunted for. Below: collapsible groups per
node kind, each nesting by `parent_id`. Drag to re-parent. Right-click for New / Favourite /
Duplicate / Delete. A node with unresolved `stale` descriptions gets a small dot. The
navigator is the tree and nothing else: the links a node has, and the ones pointing back at
it, are read and edited in the Relations tab.

#### Structure at a few hundred nodes

A creature list alone runs to hundreds of entries, and a flat tree of them is only navigable
by scrolling. Three things shape the list, none of which needs configuring first:

- **Favourites** — star a row (the star on hover, or the row's context menu) and it appears
  in a section above the tree, sorted by name. This is the reader's working set, and it is
  the one part of the structure they author.
- **Recent** — the last handful of entities opened, most recent first, excluding the one on
  screen. Appears once a project is past ~30 entities; below that everything is already
  visible and the section would only draw the same rows twice.
- **Alphabetical index** — a kind group past ~48 roots is drawn as at most twelve headings
  (`A–E`, `F–K`, …) that start closed, so a thousand characters open as a dozen rows rather
  than a thousand. The runs widen as the group grows, the index is rebuilt on whatever
  survives the filter, and a group whose names do not divide — a bulk import, a naming
  convention — is left flat, because one heading over everything hides the group and tells
  the reader nothing. Opening a node from anywhere else (palette, breadcrumb, backlink, a
  node just created) opens the heading it is filed under, exactly as jumping already opens
  collapsed ancestors.

Rows are virtualized against a fixed row height, so only the visible window is ever in the
DOM and only that window asks for thumbnails.

**None of this reaches the disk.** Sections, headings and the index are a view over
`node_list`; they add no folder, no ordering file and no front-matter field, so the project
folder stays exactly as `docs/02-data-model.md` describes it and remains legible to somebody
reading the Markdown without Wobu. Favourites are per-machine, in local storage beside the
other preferences (`store/settings.ts`), because a favourite is one reader's shortcut rather
than a fact about the world — writing them into the shared folder would sync one person's
working set onto everyone else's screen and conflict every time two people starred something.
Recent, collapse state and which headings are open are session state in `store/ui.ts`.

### Editor (fluid)
- **Sticky header** — name (inline-editable), kind badge, and the *influence breadcrumb*:
  `Vashk · Ember Guild · Cinder Bay`. Each chip is clickable and jumps to that node. This is
  the ambient reminder of where the entity sits in the hierarchy.
- **Tabs** — `Notes · References · Concepts · 3D · Relations`.
  - **Notes** is the centre of gravity: a two-column split, raw notes on the left,
    enhanced structured description on the right, with the **Enhance** button between them.
    Left is yours, right is the machine's — the spatial split teaches the model of the app.
    The right side remains visibly labelled as the machine side after hand edits; its section
    editors come from the kind registry, with swatches for palette and row editors for lists.
    Kind-specific attributes are generated from the kind registry in an initially open,
    collapsible section below Raw notes. They are human-authored Enhance inputs, so they belong
    beside notes; a permanent editor sidebar would compete with the Influence Inspector.
  - **References**: image grid; each tile carries a role chip (`silhouette`, `palette`,
    `material`, `mood`, `pose`) and a weight. Drag-drop and paste to add.
  - **Concepts**: generated art for this entity, and the only place a generation receipt is
    reachable. Hover for prompt/seed; open a tile for its immutable receipt — the recorded
    stack, params and seed, with Replay — or pin it to promote the result into References.
    The grid is virtualized and paged, so an entity with a long run stays responsive.
  - **3D**: turnaround sheets and generated meshes with an inline viewer.
  - **Relations**: the links this node has, and backlinks — "3 characters inherit from this".

### Inspector (352px) — the Influence Stack
The differentiating panel. An ordered list of layers from outermost to innermost. Each card:
colour dot + layer name + source node + fragment/reference count + **weight slider** +
**mute toggle** + expander showing the exact text and images it contributes.

Below it, the **Compiled Prompt**: the final string, with each fragment tinted in its layer's
colour, so attribution is visible at a glance. Then shot controls (output preset, aspect,
model, seed) and the Generate button.

Muting or reweighting here affects *this generation only* — it never edits the world. Editing
the world is done in the editor. That separation must be obvious.

### Status bar (26px)
Backend health, active image model, job queue depth, active LLM, last generation time.

## Secondary screens

- **Launcher** — recent projects as cover-art cards, New / Open. Shown when no project is open.
- **Forge** — the Inspector's attributed influence stack, compiled prompt, and shot controls
  promoted to full width for sustained iteration on one selected subject. Its receipt history is
  a large virtualized thumbnail grid so long runs stay responsive. Select two to four completed
  results to open their originals side by side; the originals are fetched only when that viewer
  opens. Variant grids batch one explicit axis at a time (seed, one fragment weight, preset, or
  aspect), keeping the other compiled inputs fixed and labelling each receipt with its varied axis.
  Forge also composes scenes: the selected entity is the primary history anchor, one to three more
  are chosen in prompt order, and the scene direction and aspect are explicit. Scene tiles and
  receipt details name every participant rather than presenting only the primary entity.
- **Settings** — **implemented** provider selection and local credentials for the adapters Wobu
  ships, a text-model override, the machine-local ComfyUI endpoint
  ([#108](https://github.com/krazyjakee/wobu/issues/108)), local-index inspection/rebuild,
  autosave delay, interface scale, diagnostics, version/schema information, and project wiki export.
  The earlier sketch's Replicate/fal token fields and storage-location picker are **not current
  requirements**; add a canonical issue before describing either as planned. Appearance currently
  means interface scale and the intentional dark palette, not a retained theme-switch requirement.

## Design tokens

Dark-first. Neutral cool greys so that concept art — which is the actual content — carries all
the colour. One warm accent for user actions; one violet accent reserved exclusively for
AI actions, so "the machine is about to do something" is always legible.

```
--bg           #0d0e12    --text        #e7e9f0    --accent   #e2a44f  (user actions)
--bg-panel     #14161c    --text-dim    #9aa1b3    --ai       #9d7cf5  (AI actions)
--bg-raised    #1a1d25    --text-faint  #656c7e
--border       #252932    --border-str  #333846
```

Influence layer colours — used for dots, prompt tinting, and reference borders:

```
style #e2a44f · world #4fd1c5 · species #7bd88f · culture #f28bb4 · place #6aa9f5 · subject #9d7cf5
```

Radii `6 / 10 / 14`. Base font size 13px (dense tool UI), mono for prompts. Custom Tauri
title bar so the chrome matches on all three platforms.

## Keyboard

`⌘` is Command on a Mac and Control everywhere else; both are accepted on every platform, and
only the printed form differs. These are defaults — every one of them can be changed, and the
list below is generated from the same registry the app dispatches from
(`src/store/keybindings.ts`), so it cannot drift from what the keys actually do.

**Getting around**

| | |
| --- | --- |
| `⌘K` | command palette / jump to node — works while typing, on purpose |
| `⌘/` | the keyboard reference, in the app |
| `⌘F` | filter the navigator (names and summaries; the palette searches notes too) |
| `⇧⌘L` `⌘\` `⇧⌘A` `⌘,` | Library · Forge, and back · Assets · Settings |

**Panels**

| | |
| --- | --- |
| `[` `]` | collapse navigator / inspector |
| `⇧⌘C` | collapse everything in the navigator, or expand it again |

**The editor**

| | |
| --- | --- |
| `⌘1…5` | Notes · References · Concepts · 3D · Relations (returns to the Library) |

**Writing**

| | |
| --- | --- |
| `⌘N` | new node in current group |
| `⌘Z` `⇧⌘Z` | undo / redo (`⌘Y` also redoes) |
| `⌘E` | Enhance current node |
| `⌘↵` | Generate |

`Escape` dismisses whatever is on top, `Tab` and `⇧Tab` move within a dialog without leaving it,
and the arrows plus `↵` choose in the palette and in menus. Those are fixed rather than
configurable: they are what every dialog on every platform means by those keys.

### The rules

- **One registry.** `store/keybindings.ts` declares every command, its default chord and its
  scope. Surfaces resolve a keystroke through it rather than comparing `e.key` themselves, so
  exactly one command runs for any chord — including the two that stay with their surfaces
  (Enhance and Generate know whether they are eligible; nothing else does).
- **Nothing fires while you are typing** unless the command says otherwise. Only the palette,
  the reference and the navigator filter say otherwise, because all three are ways of leaving
  where you are.
- **Nothing fires behind a dialog.** A shortcut that toggled a pane under the sheet you are
  answering acts on something you cannot see.
- **Conflicts are reported, not absorbed.** Two commands may share a chord; the earlier one in
  the registry wins, and Settings and the reference both name the winner and the command that
  has stopped working, with an offer to put the default back.
- **Bindings are per machine.** Local storage, beside the interface scale and favourites, never
  the project folder — a remapped key is a fact about one person's hands, and syncing it would
  push it onto everybody who opens the world.
