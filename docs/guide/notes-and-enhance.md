# Notes and Enhance

You write rough. Enhance turns rough into canon. The two live side by side and never overwrite each
other — left is yours, right is the machine's.

## The raw notes column

Plain Markdown, autosaved as you type. This column is **never** machine-written. Nothing Wobu does
will edit it, reorder it, or tidy it up, which means you can write badly in it on purpose:

```
kael — ex ember guild, thrown out not left
burn scar up the left side of the jaw, took the ear
wears guild plate with the signet ground off — everyone can see the grinding
too thin for the armour now
carries the lantern everywhere. never lights it.
tired. not brooding-tired. actually-tired.
```

Fragments, contradictions and half-thoughts are fine. This is input.

### Autosave

Wobu saves on a debounce — half a second by default, tunable in Settings → Editor — and on blur. The
column header shows the state: `unsaved…`, `saving…`, `saved`, or `waiting for the share…` when a
network folder has gone away and the save is being held rather than lost. If the project sits on a
slow share, nudging the delay up reduces write chatter.

## What Enhance does

The violet **Enhance** button in the editor header, or its keyboard shortcut. It reads:

- The **descriptions** of every layer above this entity — not their raw notes, so what the model
  sees is already canon.
- This entity's raw notes.
- Its attributes, and the roles of any reference images attached to it.

and returns a structured description — schema-valid JSON via tool use, not parsed prose — which
streams into the right-hand column as it arrives. You can stop it mid-stream.

> **Enhance writes nothing** Not while it streams, and not when it finishes. A completed description
> waits in memory until you accept it, and the button changes to **Review Enhance**. A run you
> stopped leaves a local draft that is never saved at all. Your existing description is untouched
> the entire time.

### Reviewing it

The right column becomes a review, section by section, with the current text and the new text side
by side and each one tagged `added`, `removed`, `changed` or `unchanged`. Per section you choose
**Keep current** or **Use new**; at the bottom you press **Accept selected**, **Accept all** or
**Reject**.

If the model left something genuinely unknown it says so rather than inventing an answer, under
**Questions for you**.

If the description you are replacing was hand-edited, Wobu refuses to write until you confirm, with
**Replace hand-edited description**. The *Current* column is re-read from disk at that moment, so
what you are comparing against is what is actually there — not what was there when Enhance started.

### Sections, not a blob

The sections depend on the entity's kind. For a character:

```
{
  "silhouette": "Tall, narrow-shouldered, forward-canted stance. Armour hangs
                 wrong — plate cut for a broader frame, cinched with rope.",
  "anatomy":    "Burn scar from left jaw to temple; left ear absent, the
                 cartilage melted to a ridge.",
  "costume":    "Ember Guild kiln-glaze plate, signet ground flat at the
                 collarbone leaving a bright abraded oval.",
  "materials":  "Ash-glazed ceramic over oiled leather; rope, not buckles.",
  "palette":    ["#2b2118", "#c2703a", "#7fa5a3"],
  "signature":  ["Unlit ashglass lantern at the belt",
                 "Ground-off guild signet"],
  "never":      ["Modern firearms", "Symmetrical face", "Clean armour"]
}
```

Structure is what makes the prompt compiler useful. A *material study* preset can boost `materials`
and drop `silhouette`; a *turnaround* does the reverse. Neither is possible against a paragraph of
prose. `never` becomes negative prompt input, and `palette` can drive a colour-conditioning pass.

## The four rules Enhance follows

These constraints matter more than any prompt wording, and knowing them tells you how to write notes
that enhance well.

| | |
| --- | --- |
| Do not invent | Enhance elaborates on what your notes imply. Where something is missing it asks you a question rather than confabulating an answer. |
| Write visually | Every sentence must change what a renderer would draw. History, motive and plot are dropped unless they are visible on the body — "thrown out of the guild" survives only as the ground-off signet. |
| Do not restate inherited traits | If the species already establishes four-jointed legs, the character description will not repeat them. Only *deviations* from the inherited baseline get written down. |
| Populate *Never* | Explicit negatives are the main defence against visual drift, so Enhance always fills them in. |

> **Why Enhance reads the whole stack** The third rule is the subtle one, and it is the reason
> Enhance is given every layer above the entity rather than just the entity's own notes. Each layer
> stays lean and orthogonal, nothing is said twice, and the compiled prompt stays inside the model's
> budget. An entity enhanced in isolation would duplicate half its species.

## Freshness and staleness

| State | Shown as | Meaning |
| --- | --- | --- |
| `none` | not generated | Never enhanced. The right column explains what would happen. |
| `enhancing` | enhancing… | Streaming right now. Stoppable. |
| `fresh` | fresh | Enhanced, and everything it was based on is unchanged. |
| `edited` | edited by you | You changed the machine's text by hand. Fully supported. |
| `stale` | stale | The notes, or something upstream, changed since the last enhance. |

On accept, Wobu stamps which upstream sources were used. When any of them changes later, this entity
flips to **stale** and picks up a quiet dot in the navigator. Nothing regenerates on its own —
re-enhancing spends your money and might overwrite an edit you made deliberately, so it stays your
call.

## Order of operations

Enhance **top down**. Art Style first, then World Canon, then species, cultures, settings, and
subjects last. Because each enhance reads the descriptions above it, an un-enhanced parent means a
child is enhanced against nothing — and you will feel it in the output.

> **Editing the machine's column** The right column is yours to edit too, field by field, including
> the palette swatches and the list sections. Accepting an enhance is an ordinary edit as far as the
> rest of the app is concerned, so undo takes it back. The asymmetry is only in the other direction:
> Wobu never touches the left column.
