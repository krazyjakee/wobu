# Entities and hierarchy

Species, characters, places, props, cultures, vehicles — and the Art Style and World Canon pages
themselves — are all the same thing underneath. Wobu calls each one an **entity**, and they differ
only in **kind**. A kind picks the icon, the colour, where it sits in a prompt, which sections its
description has, and what it can be joined to.

That is why there is only one editor to learn. It is also why adding "Factions" or "Technologies"
later is a small job rather than a whole new feature.

## The ten kinds

Wobu never invents a kind that is not on this list. *Nests* means one can go inside another of the
same kind — a region inside a region. Nesting never crosses kinds; joining things up across kinds is
what links are for.

| Kind | Nests | Description sections |
| --- | --- | --- |
| Art Style | one per world | Medium, Rendering, Line quality, Lighting model, Palette, Never |
| World Canon | one per world | Era, Tone, Tech & magic level, Materials, Palette, Never |
| Species | yes | Silhouette, Anatomy, Materials, Palette, Signature details, Never |
| Culture | yes | Costume, Ornament, Iconography, Weapon language, Materials, Palette, Never |
| Setting | yes | Climate, Architecture, Ambient light, Wear & age, Materials, Palette, Never |
| Character | no | Silhouette, Anatomy, Costume, Materials, Palette, Signature details, Never |
| Creature | no | Silhouette, Anatomy, Materials, Palette, Signature details, Never |
| Prop | yes | Silhouette, Materials, Wear & age, Palette, Signature details, Never |
| Environment | no | Architecture, Ambient light, Climate, Materials, Palette, Signature details, Never |
| Vehicle | no | Silhouette, Materials, Wear & age, Palette, Signature details, Never |

Every kind has a **Never** section, because anything can have something it should never look like.
*Palette*, *Signature details* and *Never* are lists; the rest is ordinary writing.

Those sections are what make Enhance worth having. Because the description has a *Silhouette* field
and a *Materials* field, Wobu can lean on the right ones for a full-body shot and different ones for
a close study of fabric. A single lump of prose could not do that.

Kinds also carry a few short facts you fill in yourself rather than have written for you — *Era*,
*Scale*, *Biome* and *Primary material*, on the kinds where they make sense. They sit under your
notes, and a kind with none of them shows nothing.

## Two ways things connect

Wobu keeps **nesting** and **linking** apart, and the difference matters.

### Nesting — inside the same kind

What something sits inside is always the same kind as itself. Ember Coast holds Cinder Bay holds the
Kiln Quarter. Vashk has a sub-species. A lantern belongs to a set of props. This is the shape the
list on the left draws, and you change it by dragging. Only Species, Culture, Setting and Prop nest;
the other six are flat.

Whatever something sits inside counts fully towards it. The Kiln Quarter inherits everything Cinder
Bay says, which inherits everything Ember Coast says.

### Linking — across kinds

Links are how influence travels between different sorts of thing. Each one has a job and a strength,
and can be switched off. You set them on the **Relations** tab, which also lists everything pointing
back *at* this page — which is how you find out what you are about to break before you go editing a
species.

| Role | Shown as | Means |
| --- | --- | --- |
| `styled_by` | Styled by | Drawn in this art style. Art Style is included in everything anyway. |
| `species_of` | Species | This character or creature is one of those. Feeds the Ancestry layer. |
| `member_of` | Member of | Belongs to that culture, guild or faction. |
| `located_in` | Located in | Lives in, or is found in, that place. |
| `related_to` | Related to | A sideways connection — a rival, a matching prop, a sister design. It adds to the subject rather than getting a layer of its own. |

## What a page holds

| | |
| --- | --- |
| Name and summary | The summary is one line, shown on rows in the list and on layer cards. |
| Raw notes | Yours. Never machine-written. |
| Description | The tidied sections from Enhance. You can edit them. |
| Description state | `none · enhancing · fresh · edited · stale` |
| Attributes | The short facts for this kind — era, scale, biome, main material. |
| Tags | Whatever you like, for filtering your pictures and the website export. |
| Cover picture | The image shown on cards, on list rows and in the right-hand panel. |
| References | Pictures attached here, each with a job and a strength. |
| Links | The connections above. |
| Locked seed and LoRA | Picture-making settings that belong to this page — see [Generating](generating.md). |

### Going stale

A description goes **stale** when your notes changed, or when something above it changed, since the
last time you enhanced. Wobu shows a quiet dot and offers to redo it — it never quietly redoes it
itself, because that would spend your money and might throw away an edit you made by hand.

**Edited by you** means you changed the machine's description yourself. That is completely fine.
Enhancing over it shows you a comparison first and refuses to write anything until you agree.

## Making and organising

- **New entity** at the bottom of the list, from **Jump to…**, or right-click → **New**. You choose
  a kind, a name, and — for kinds that nest — what it goes inside.
- **Duplicate** copies the notes, description and links. It is the quickest way to make the second
  of anything. Art Style and World Canon cannot be duplicated.
- **Drag to move**, within a kind. This changes what a page inherits, so it will often mark things
  below it as stale. Dropping onto a group heading puts it back at the top level — always allowed,
  so anything filed in the wrong place always has a way out.
- **Delete** asks first, then removes one text file. **Anything that was inside it moves up** rather
  than being deleted too, and links pointing at it are cleaned out of the other files. Pictures made
  from it stay where they are — that history is never rewritten.
- **Undo** covers all of this. It only lasts for as long as the app is open, on purpose: between one
  session and the next the folder may have been edited elsewhere, synced or restored from a backup,
  and replaying a week-old undo over that would not be undo. One thing is said out loud when you
  undo a delete — the links that pointed at it were removed by the delete, and they do not come
  back.

> **Getting the level right** The single most common mistake is writing something in the wrong
> place. If it is true of every Vashk, it belongs on the species. If it is true only of guild
> members, it belongs on the culture. If it is true only of Kael, it belongs on Kael.
>
> Too low and you repeat yourself and drift. Too high and it leaks into everything. When you are not
> sure, ask: *would this be wrong for the one next to it?* If yes, it is too high.
