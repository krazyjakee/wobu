# Entities and hierarchy

Species, characters, settings, props, cultures, vehicles — and the Art Style and World Canon
entities themselves — are all the same record type. They differ only in kind, which selects an icon,
a colour, an influence layer, a set of description sections and some default relations.

This is why there is exactly one editor to learn. It is also why adding "Factions" or "Technologies"
later is a registry change rather than a new feature.

## The ten kinds

The list of kinds lives in Wobu itself, and the interface never invents one that is not on it.
`Nests` means the kind can be nested inside *itself* — a region inside a region. Nesting never
crosses kinds; that is what links are for.

| Kind | Nests | Description sections |
| --- | --- | --- |
| Art Style | singleton | Medium, Rendering, Line quality, Lighting model, Palette, Never |
| World Canon | singleton | Era, Tone, Tech & magic level, Materials, Palette, Never |
| Species | yes | Silhouette, Anatomy, Materials, Palette, Signature details, Never |
| Culture | yes | Costume, Ornament, Iconography, Weapon language, Materials, Palette, Never |
| Setting | yes | Climate, Architecture, Ambient light, Wear & age, Materials, Palette, Never |
| Character | no | Silhouette, Anatomy, Costume, Materials, Palette, Signature details, Never |
| Creature | no | Silhouette, Anatomy, Materials, Palette, Signature details, Never |
| Prop | yes | Silhouette, Materials, Wear & age, Palette, Signature details, Never |
| Environment | no | Architecture, Ambient light, Climate, Materials, Palette, Signature details, Never |
| Vehicle | no | Silhouette, Materials, Wear & age, Palette, Signature details, Never |

Every kind has a **Never** section, because every kind can contribute to the negative prompt.
*Palette*, *Signature details* and *Never* are lists; everything else is prose.

The sections are what make Enhance useful. A structured description with a *Silhouette* field and a
*Materials* field lets the prompt compiler pick the right fields for a full-body shot versus a
material study. A blob of prose could not.

Kinds also carry a few small **attributes** you write yourself rather than have enhanced: *Era*,
*Scale*, *Biome* and *Primary material*, on the kinds where they mean something. They live under the
raw notes, and a kind that declares none shows nothing.

## Two kinds of relationship

Wobu distinguishes **nesting** from **linking**, and the difference matters.

### Nesting — within a kind

An entity's parent is always the same kind as itself. Ember Coast contains Cinder Bay contains the
Kiln Quarter. Vashk has a sub-species. A lantern belongs to a prop set. Nesting is what the
navigator tree draws, and you change it by dragging. Only Species, Culture, Setting and Prop nest;
the other six are flat.

A parent is treated as an implicit influence of full weight. The Kiln Quarter inherits everything
Cinder Bay says, which inherits everything Ember Coast says.

### Linking — across kinds

Links are the edges the influence engine walks. Each has a role and a weight, and can be disabled.
You set them from the **Relations** tab, which also lists the links pointing *back* at this entity —
which is how you find out what you are about to break before you edit a species.

| Role | Shown as | Means |
| --- | --- | --- |
| `styled_by` | Styled by | Rendered in this art style. The Art Style entity is seeded into every stack anyway. |
| `species_of` | Species | This character or creature is of that species. Feeds the Ancestry layer. |
| `member_of` | Member of | Belongs to that culture, guild or faction. |
| `located_in` | Located in | Lives in, or is found in, that setting. |
| `related_to` | Related to | A lateral association — a rival, a matching prop, a sibling design. It contributes at the subject layer rather than getting a layer of its own. |

## What an entity holds

| | |
| --- | --- |
| Name and summary | The summary is one line, shown on tree rows and influence cards. |
| Raw notes | Your Markdown. Never machine-written. |
| Description | The structured, enhanced sections. Editable. |
| Description state | `none · enhancing · fresh · edited · stale` |
| Attributes | Kind-specific fields — era, scale, biome, primary material. |
| Tags | Free-form, for filtering the asset library and the wiki export. |
| Cover asset | The image shown on cards, on tree rows and in the inspector. |
| References | Images attached to this entity, each with a role and a weight. |
| Links | The influence edges above. |
| Locked seed and LoRA | Generation state that belongs to this entity — see [Generating](generating.md). |

### Description state, and staleness

A description goes **stale** when the raw notes changed, or when something upstream changed, since
the last Enhance. Wobu shows a quiet dot and a re-enhance affordance — it never silently
regenerates, because regenerating costs your money and might undo an edit you made by hand.

**Edited by you** means you changed the machine's description yourself. That is fully supported;
re-enhancing over it offers a diff and refuses to write until you say so explicitly.

## Creating and organising

- **New entity** at the bottom of the navigator, the command palette, or right-click → **New**. You
  pick a kind, a name and — for kinds that nest — an optional parent.
- **Duplicate** copies notes, description and links. It is the fastest way to make the second of
  anything. Singletons cannot be duplicated.
- **Drag to re-parent**, within a kind. This changes what the entity inherits, so it will often mark
  descendants stale. Dropping onto a group header moves an entity back to the top level — which is
  always allowed, so a wrongly parented entity always has a way out.
- **Delete** asks for confirmation and removes one Markdown file. **Its children are promoted to its
  parent** rather than deleted with it, and links pointing at the deleted entity are stripped from
  every other file. Generations that referenced it stay on disk — they are write-once history.
- **Undo** covers all of this. It is session-only and never persisted: between quit and relaunch the
  folder may have been edited in Obsidian, pulled, synced or restored, and replaying a week-old
  inverse over that would not be undo. One caveat is stated out loud when you undo a delete — the
  links that pointed at the entity were removed by the delete and do not come back.

> **Getting the altitude right** The single most common mistake is writing at the wrong level. If a
> detail is true of every Vashk, it belongs on the species. If it is true only of guild members, it
> belongs on the culture. If it is true only of Kael, it belongs on Kael.
>
> Put it too low and you repeat yourself and drift. Put it too high and it contaminates everything.
> When unsure, ask: *would this be wrong for a sibling entity?* If yes, it is too high.
