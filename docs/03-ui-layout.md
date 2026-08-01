# 03 — UI Layout

## Shape of the app

One primary screen — **the Workspace** — with three vertical regions plus a mode rail. Users
live here 95% of the time. Everything else is a mode swap in the centre or a modal.

```
┌────────────────────────────────────────────────────────────────────────────────────┐
│  wobu   [ Ashfall ▾ ]   Species / Vashk / Kael Vantris        ⌘K search      ⚙︎     │ title bar
├────┬────────────────────┬──────────────────────────────┬───────────────────────────┤
│ ▤  │ ⌕ filter…          │  KAEL VANTRIS      ⋯  Enhance│  INFLUENCE STACK          │
│ ▦  │                    │  character · Vashk · Ember G.│  ┌──────────────────────┐ │
│ ✦  │ ★ Art Style        │ ─────────────────────────────│  │ ● Art Style          │ │
│ ⬚  │ ★ World Canon      │  Notes │ Refs │ Concepts │ 3D│  │   Ashfall House  1.0 ▸│ │
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
Icon-only, always visible. `Library` (the tree above) · `Board` (mood-board canvas) ·
`Forge` (full-width generation + result grid) · `Assets` (all images, filterable) ·
`Settings`. Keeps the top bar clean and makes mode switching muscle memory.

### Navigator (272px, resizable)
Filter box at top. Two pinned singletons — **Art Style** and **World Canon** — sit above the
rule, because they are the roots of every influence stack and should never be hunted for.
Below: collapsible groups per node kind, each nesting by `parent_id`. Drag to re-parent.
Right-click for New / Duplicate / Delete. A node with unresolved `stale` descriptions gets a
small dot.

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
  - **Concepts**: generated art for this entity. Hover for prompt/seed; pin to promote a
    result into References.
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
- **Forge** — the Inspector's controls promoted to full width with a large result grid, for
  when you're iterating on one subject and want to compare 20 variants.
- **Board** — freeform pan/zoom canvas for mood boarding; images can be dragged onto a node
  to become references.
- **Settings** — providers (Anthropic key, ComfyUI URL, Replicate/fal tokens), model defaults,
  storage location, appearance.

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

| | |
| --- | --- |
| `⌘K` | command palette / jump to node |
| `⌘N` | new node in current group |
| `⌘E` | Enhance current node |
| `⌘↵` | Generate |
| `⌘1…5` | editor tabs |
| `[` `]` | collapse navigator / inspector |
| `⌘\` | toggle Forge |
