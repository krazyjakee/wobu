# The workspace

One primary screen with three vertical regions and a mode rail. You will live here most of the time
— everything else is a mode swap in the centre or a dialog.

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  wobu   [ Ashfall ▾ ]   Species / Vashk / Kael Vantris     ⌕ Jump to…      ⚙︎    │ title bar
├────┬────────────────────┬────────────────────────────┬───────────────────────────┤
│ ▤  │ ⌕ Filter world…    │ KAEL VANTRIS   saved Enhance│  INFLUENCE STACK         │
│ ▦  │ 812 entities  ⌄all │ character · inherits Vashk ·│  ┌──────────────────────┐│
│ ✦  │                    │─────────────────────────────│  │ ● Art Style      1.0 ▸││
│    │ ★ Art Style        │ Notes │ Refs │ Concepts │ 3D│  ├──────────────────────┤│
│ ⚙  │ ★ World Canon      │       │      │          │ Rel│ │ ● World Canon    0.8 ▸││
│    │                    │─────────────────────────────│  ├──────────────────────┤│
│    │ ▾ Favourites       │ ┌───────────┬──────────────┐│  │ ● Ancestry · Vashk   ▸││
│    │ ▾ Recent           │ │ RAW NOTES │ ENHANCED     ││  ├──────────────────────┤│
│    │ ▾ Species          │ │  yours    │ from Enhance ││  │ ● Culture · Ember G. ▸││
│    │    Vashk           │ │           │ Silhouette   ││  ├──────────────────────┤│
│    │ ▾ Cultures         │ │ scarred   │ Anatomy      ││  │ COMPILED PROMPT       ││
│    │    Ember Guild     │ │ ex-guild  │ Materials    ││  │ ▓▓▓▓ tinted by layer  ││
│    │ ▾ Settings         │ │ …         │ Palette      ││  ├──────────────────────┤│
│    │  ▾ Ember Coast     │ │           │ Signature    ││  │ Preset ▾   Aspect ▾   ││
│    │     Cinder Bay     │ │           │ Never        ││  │ Seed · Variant grid   ││
│    │ ▾ Characters       │ └───────────┴──────────────┘│  │    ✦ Generate         ││
│    │    Kael Vantris ◀  │                             │  └──────────────────────┘│
│    │ + New entity       │                             │                          │
├────┴────────────────────┴─────────────────────────────┴──────────────────────────┤
│ Ashfall · /vol/Ashfall.wobu · 812 entities · ComfyUI connected · queue 0 · ⏱4.2s  │ status
└──────────────────────────────────────────────────────────────────────────────────┘
   rail          navigator                 editor                    inspector
```

## Mode rail

Icon-only, always visible on the far left. Four destinations, which keeps the top bar clean and
makes switching muscle memory. Each button's tooltip carries its current shortcut, read live from
the keyboard registry — so if you rebind it, the tooltip moves with it.

| | |
| --- | --- |
| Library | The world tree and the entity editor — the screen above. Your default. |
| Forge | Generation promoted to full width with a large result grid, for iterating hard on one subject. |
| Assets | Every image in the project in one filterable grid, with what each one is attached to. |
| Settings | Providers, keys, legal, agent access, storage, editor, appearance, keyboard, wiki export, diagnostics, about and licences. |

The navigator and the inspector belong to Library. In Forge, Assets and Settings they are not
collapsed but absent — the mode gets the whole width.

## Title bar

A custom title bar, so the chrome matches on Linux, macOS and Windows. Left to right: the **wobu**
mark, the **project menu** (click the project name for its full path, **Share this project…** and
**Close project**), a **read-only** badge when the folder is not writable, and the **influence
breadcrumb** for the selected entity. Every crumb is clickable and jumps to that entity. On the
right, **Jump to…** opens the command palette, and the gear opens Settings.

## Navigator

Resizable by dragging its right edge, and collapsible from the keyboard.

- **Filter box** at the top — placeholder *Filter world…* — matches entity names and one-line
  summaries. For searching inside notes and descriptions, use the command palette instead; the
  navigator says so when nothing matches.
- **A count and a collapse control.** `812 entities`, or `14 of 812 shown` while filtering, and one
  button that reads **Collapse all** or **Expand all**. Collapsing closes every kind group and
  leaves the state *inside* them alone, so re-opening a group returns you to the shape you had.
- **Two pinned singletons** — Art Style and World Canon — sit above the rule, because they root
  every influence stack.
- **Favourites** and **Recent** sections come first. Favourites are the rows you starred, listed
  alphabetically; Recent is the handful of entities you have opened lately and appears only in
  worlds large enough to need it. Both are shortcuts into the tree below, not a second copy of it —
  you cannot drag them.
- **A group per kind**, each nesting by parent. Regions contain cities contain districts.
- **Alphabetical bands** appear inside a group once it has too many top-level rows to scan — `A–E`,
  `F`, `M`, and `#` for everything that does not start with a letter. They start closed.
- **A thumbnail per row**, so a species you have already drawn is recognisable without reading. Rows
  with no picture keep the same slot, filled with the kind's icon, so nothing shifts.
- **Drag to re-parent.** Dropping an entity onto another moves it in the hierarchy — and therefore
  changes what it inherits. Only within the same kind; dropping onto a group header moves it to the
  top level.
- **Right-click** for New, New child, favourite, Duplicate and Delete. Singletons cannot be
  duplicated or deleted.
- **Status dots.** An entity whose description has gone stale gets a small dot; an entity someone
  else has open in another copy of Wobu gets a presence dot.

> **Broken files** If a file in the project folder cannot be parsed — a sync client mangled it, or a
> hand edit broke the frontmatter — Wobu lists it at the top of the navigator with the parser's own
> error and offers **Show in folder** and **Reload**. It never silently skips a file, and it never
> writes over one it could not read.

## Editor

The fluid centre column, and the place you actually author.

### Header

An inline-editable name, a kind badge, a *stale* badge when the description has drifted from the
notes, the **inherits** line listing the layers above this entity, the autosave state, and the
**Enhance** button in the violet AI accent. Wobu saves as you type on a delay you can tune in
Settings → Editor.

### Tabs

Five tabs, each with a shortcut shown in its tooltip:

| | |
| --- | --- |
| Notes | The centre of gravity: raw notes on the left, the enhanced structured description on the right. |
| References | Image grid; each tile carries a role, a weight, a mute toggle and a cover control. Drag, drop or paste to add. |
| Concepts | Generated art for this entity, with the prompt and seed behind each result. Pin one to promote it into References. |
| 3D | Turnaround review and generated meshes, with an inline viewer. |
| Relations | The links this entity has and the ones pointing back at it. |

### The Notes split

Two columns. **Left is yours, right is the machine's** — the left is tagged *yours* and the right
*written by Enhance*. That spatial split is the mental model of the whole app: Wobu never writes
into your column, and you can always edit its column. Covered in [Notes and
Enhance](notes-and-enhance.md).

## Inspector

On the right, collapsible. This is the differentiating panel, and it has no tabs — three sections
stacked in one column:

1. **Influence stack** — the layers feeding this generation, outermost first, with the provider's
   reference budget across the top.
2. **Compiled prompt** — the exact text that will be sent, with a **Show sources** toggle that tints
   every fragment by the layer that produced it.
3. **Shot controls** — output preset, aspect, model, an extra shot prompt, seed, the variant grid,
   the project's spend ceiling, and **Generate**.

Each layer card carries a thumbnail or a colour dot, the source entity, a count of the text and
image fragments it contributed and how many were dropped, a weight slider, a mute toggle and an
**Open source** button. The full explanation is in [The influence stack](influence.md).

> **The one rule worth internalising** Muting or reweighting in the inspector affects **this
> generation only**. It never edits your world. Editing the world happens in the editor. If you find
> yourself muting the same layer every time, that is a signal to go and fix the entity.

## Status bar

Along the bottom in every mode, and mostly read-only: project name and path, whether the folder is a
network share or read-only, sync state and connected peers, how many other people have it open, the
entity count, which panels are hidden, provider health and the active image model, the job queue
depth, the text model and its context size, and a timer for the running generation.

Two things in it are buttons. **queue n** switches to Forge, and **notifications** opens the
notification centre.

### The notification centre

Toasts vanish; some failures should not. Anything that failed — a generation the provider refused, a
save the share rejected, a job that cost money and produced nothing — is kept here with what it
cost, what went wrong in the provider's own words, and where to go next: **Open Settings**, **Open
in Forge** or **Show the entity**. Opening the panel marks everything read; **Clear all** empties
it.

## Command palette

From anywhere, and deliberately reachable mid-sentence — it is how you leave where you are. It
searches in two phases: entity names and summaries match instantly, then full-text hits from inside
notes and descriptions arrive a moment later, ranked by the search index. Results are grouped
**Entities**, **In notes and descriptions** and **Commands**. Arrow keys move, Enter picks, Escape
closes.

The commands are New entity, Undo, Redo, Toggle navigator, Toggle inspector and Keyboard shortcuts.
On a read-only folder the ones that write are absent rather than disabled.

## Pickers

Anywhere Wobu has more than a handful of choices — the output preset, an aspect, a relation target,
a parent entity, an asset filter — the control is a searchable combobox rather than a dropdown. Type
to filter, arrows to move, Enter to take, Escape to abandon the search and keep what you had. It
announces how many results it has, options that are offered but unavailable say so rather than
disappearing, and lists of thousands of entities stay responsive because only the visible rows are
drawn. Genuine five-item enums are still plain dropdowns.

## Appearance

Settings → Appearance carries a **Theme** — *Match system*, *Light* or *Dark* — and an **Interface
scale**. Both themes carry the same palette: the six influence-layer colours stay distinguishable
from one another, including under the common forms of colour-vision deficiency, and text keeps its
contrast either way. *Match system* follows the desktop as it changes rather than freezing whatever
it was when you chose it.

The interface scale scales the whole interface rather than only the text, because the navigator and
inspector are fixed widths and type alone would grow inside boxes that did not.
