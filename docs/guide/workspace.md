# The workspace

One main screen, split into three columns, with a strip of buttons down the left. This is where you
will spend nearly all your time — everything else is either a swap of the middle column or a dialog
box.

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

## The four screens

The icons down the far left are always there. Only four, so switching becomes muscle memory
quickly. Hover one and its tooltip tells you the current keyboard shortcut — and if you change that
shortcut, the tooltip changes with it.

| | |
| --- | --- |
| Library | Your world and the page you are writing — the screen above. This is home. |
| Forge | Making pictures, given the whole window, with a big grid of results. For when you are hammering away at one subject. |
| Assets | Every picture in the world in one grid you can filter, showing what each one is attached to. |
| Settings | Keys, legal, AI assistants, storage, editing, appearance, keyboard, website export, diagnostics, about and licences. |

The world list and the right-hand panel belong to Library. In the other three they are not squashed
up, they are simply gone — those screens get the whole window.

## Title bar

Wobu draws its own, so it looks the same on Windows, macOS and Linux. Left to right: the **wobu**
mark, then the **world menu** (click the name for its full location, **Share this project…** and
**Close project**), a **read-only** badge if you cannot write to the folder, and the trail of pages
above whoever you have open. Every step in that trail is clickable. Over on the right, **Jump to…**
opens the search box and the cog opens Settings.

## Navigator

The list of everything in your world, down the left. Drag its right edge to resize it, or hide it
from the keyboard.

- **Filter box** at the top — *Filter world…* — matches names and one-line summaries. To search
  inside notes and descriptions, use **Jump to…** instead; the list tells you so when nothing
  matches.
- **A count, and a way to fold it up.** `812 entities`, or `14 of 812 shown` while filtering, and
  one button reading **Collapse all** or **Expand all**. Collapsing shuts the top-level groups and
  leaves whatever you had open *inside* them alone, so opening one again puts you back where you
  were.
- **Art Style and World Canon** sit above the line, on their own, because everything is built on
  them.
- **Favourites** and **Recent** come next. Favourites are the rows you starred; Recent is what you
  have opened lately, and only turns up in worlds big enough to need it. Both are shortcuts into
  the list below rather than a second copy of it, so you cannot drag things around in them.
- **A group per sort of thing**, each nested by what it belongs to. Regions hold cities hold
  districts.
- **Letter bands** appear inside a group once it has more rows than you can scan — `A–E`, `F`, `M`,
  and `#` for anything not starting with a letter. They start closed.
- **A thumbnail on every row**, so a species you have already drawn is recognisable without reading.
  Rows without a picture keep the same space, filled with an icon, so nothing jumps about.
- **Drag to move things.** Dropping one page onto another moves it inside — and changes what it
  inherits. Only within the same sort of thing; dropping onto a group heading puts it back at the
  top level.
- **Right-click** for New, New child, favourite, Duplicate and Delete. Art Style and World Canon
  cannot be copied or deleted.
- **Little dots.** A page whose description has fallen behind gets one; so does a page somebody else
  currently has open.

> **When a file is broken** If something in the folder cannot be read — a sync app mangled it, or a
> hand edit went wrong — Wobu lists it at the top with the actual error, and offers **Show in
> folder** and **Reload**. It never quietly skips a file, and it never writes over one it could not
> read.

## Editor

The middle column, and where you actually do the writing.

### Header

The name, which you can click and type over; a badge saying what sort of page it is; a *stale* badge
when the description has fallen behind the notes; the **inherits** line showing what sits above this
page; whether it has saved; and the **Enhance** button in violet. Wobu saves as you type, after a
short pause you can adjust in Settings → Editor.

### Tabs

Five of them, each with its shortcut in the tooltip:

| | |
| --- | --- |
| Notes | The heart of it: your rough notes on the left, the tidied description on the right. |
| References | Your reference pictures. Each one has a job, a strength, a mute switch and a "use as cover" button. Drag, drop or paste to add more. |
| Concepts | Pictures made for this page, each with the prompt and seed behind it. Pin one to promote it to a reference. |
| 3D | Turnaround sheets and the rough 3D shapes made from them, with a viewer built in. |
| Relations | What this page is joined to, and what is joined to it. |

### The two columns

**Left is yours, right is the machine's** — the left is labelled *yours* and the right *written by
Enhance*. That split is the whole idea of the app in one picture: Wobu never writes in your column,
and you can always edit its column. More in [Notes and Enhance](notes-and-enhance.md).

## Inspector

The panel on the right, which you can hide. This is the part of Wobu that does not exist anywhere
else, and it has no tabs — just three things stacked in a column:

1. **Influence stack** — every layer feeding this picture, broadest first, with a strip at the top
   showing how many reference pictures the service will take.
2. **Compiled prompt** — the exact words that will be sent, with a **Show sources** switch that
   tints every phrase by the page it came from.
3. **The shot controls** — what sort of sheet, what shape, which model, an extra line of direction,
   the seed, the variant grid, and **Generate**.

Each layer card shows a thumbnail or a coloured dot, which page it is, how many bits of text and
how many pictures it put in (and how many did not fit), a strength slider, a mute switch and an
**Open source** button. Full explanation in [The influence stack](influence.md).

> **The one thing worth remembering** Muting or turning down a layer here changes **this picture
> only**. It never edits your world. Editing your world happens in the middle column. If you find
> yourself muting the same layer every time, that is a hint: go and fix the page.

## Status bar

Along the bottom of every screen, and mostly just telling you things: the world's name and where it
is, whether the folder is on a network drive or read-only, sync status and who else is connected,
how many people have it open, how many pages there are, which panels are hidden, whether your AI
services are reachable and which image model is in use, how many jobs are waiting, the text model
and how much it can hold, and a timer while a picture is being made.

Two things there are buttons. **queue n** takes you to Forge, and **notifications** opens the list
of things that went wrong.

### Notifications

Pop-up messages disappear; some failures should not. Anything that failed — a picture the service
refused, a save the shared drive would not take, a job that cost money and produced nothing — is
kept here with what it cost, what went wrong in the service's own words, and somewhere to go next:
**Open Settings**, **Open in Forge** or **Show the entity**. Opening the panel marks them all read;
**Clear all** empties it.

## Jump to…

Available from anywhere, and it works even mid-sentence, because it is how you leave where you are.
It searches in two goes: names and summaries match instantly, then matches from inside your notes
and descriptions arrive a moment later. Results are grouped under **Entities**, **In notes and
descriptions** and **Commands**. Arrow keys move, Enter picks, Escape closes.

The commands are New entity, Undo, Redo, Toggle navigator, Toggle inspector and Keyboard shortcuts.
On a read-only folder, the ones that would write something are simply not there rather than greyed
out.

## Choosing from a long list

Anywhere Wobu has more than a handful of options — the sort of sheet, the shape, what to join
something to, which parent, a filter — you get a box you can type in rather than a plain dropdown.
Type to narrow it, arrows to move, Enter to take one, Escape to give up and keep what you had. It
says how many matches it found, options that exist but are unavailable say so instead of vanishing,
and a list of thousands stays quick because only the rows you can see are drawn. Genuinely short
lists are still plain dropdowns.

## Theme and size

Settings → Appearance has a **Theme** — *Match system*, *Light* or *Dark* — and an **Interface
scale**. Both themes use the same colours: the six layer colours stay tellable apart, including for
the common sorts of colour blindness, and text stays readable either way. *Match system* keeps
following your desktop as it changes, rather than freezing whatever it was when you picked it.

Interface scale grows the whole app, not just the text — the two side panels are fixed widths, so
text alone would end up straining against boxes that had not grown with it.
