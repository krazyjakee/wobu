# Notes and Enhance

You write roughly. Enhance turns rough into something usable. The two sit side by side and never
write over each other — left is yours, right is the machine's.

## Your notes, on the left

Plain text, saved as you type. This column is **never** written to by Wobu. Nothing the app does
will edit it, reorder it or tidy it up, which means you are free to write badly in it on purpose:

```
kael — ex ember guild, thrown out not left
burn scar up the left side of the jaw, took the ear
wears guild plate with the signet ground off — everyone can see the grinding
too thin for the armour now
carries the lantern everywhere. never lights it.
tired. not brooding-tired. actually-tired.
```

Half-thoughts, contradictions, single words. All fine. This is raw material.

### Saving

Wobu saves shortly after you stop typing — half a second by default, adjustable in Settings →
Editor — and whenever you click away. The column header tells you where it is up to: `unsaved…`,
`saving…`, `saved`, or `waiting for the share…` when a network drive has gone quiet and your save is
being held rather than thrown away. On a slow shared drive, nudging that delay up means fewer
writes.

## What Enhance does

The violet **Enhance** button at the top of the editor, or its keyboard shortcut. It reads:

- The **descriptions** of every page above this one — the tidied versions, not their rough notes, so
  what it sees is already settled.
- This page's notes.
- Its short facts, and what job each reference picture on it has.

and sends back a description split into sections, which appears in the right-hand column as it is
written. You can stop it part way through.

> **Enhance saves nothing** Not while it is writing, and not when it finishes. A finished
> description waits for you, and the button changes to **Review Enhance**. If you stop it half way,
> what it managed is kept on screen and never written to disk at all. Your existing description is
> untouched the whole time.

### Reading it over

The right column turns into a comparison, section by section, with what you have now beside what has
just been written, and each one marked `added`, `removed`, `changed` or `unchanged`. For each
section you pick **Keep current** or **Use new**; at the bottom you press **Accept selected**,
**Accept all** or **Reject**.

If something was genuinely unclear it says so rather than making it up, under **Questions for you**.

If the description you are about to replace was one you had edited by hand, Wobu will not write over
it until you confirm, with **Replace hand-edited description**. The *Current* column is re-read from
the folder at that moment, so what you are comparing against is what is really there — not what was
there when you pressed the button.

### Sections, not a blob

Which sections you get depends on the kind of page. For a character:

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

Splitting it up is what makes the rest work. A close study of materials can lean on `materials` and
push `silhouette` back; a turnaround does the opposite. Neither is possible with one paragraph of
prose. `never` becomes the "do not draw this" half of the prompt, and `palette` can be used to steer
the colours.

## The four rules Enhance follows

These matter more than any clever wording, and knowing them tells you how to write notes that
enhance well.

| | |
| --- | --- |
| Never make things up | Enhance builds on what your notes imply. Where something is missing it asks you rather than inventing an answer. |
| Only write what can be seen | Every sentence has to change what would be drawn. History, motives and plot are dropped unless they show on the body — "thrown out of the guild" survives only as the ground-off signet. |
| Never repeat what is inherited | If the species already says four-jointed legs, the character will not say it again. Only the ways this one *differs* get written down. |
| Always fill in *Never* | Saying what should not appear is the main defence against everything slowly drifting, so Enhance always writes some. |

> **Why Enhance reads the whole chain** The third rule is the subtle one, and it is why Enhance is
> handed every page above this one rather than just this page's notes. Each layer stays lean and
> stays in its lane, nothing is said twice, and the prompt stays short enough to fit. A page
> enhanced on its own would repeat half its species.

## Fresh and stale

| State | Shown as | Meaning |
| --- | --- | --- |
| `none` | not generated | Never enhanced. The right column explains what would happen. |
| `enhancing` | enhancing… | Being written right now. You can stop it. |
| `fresh` | fresh | Enhanced, and nothing it was based on has changed since. |
| `edited` | edited by you | You changed the machine's text by hand. Perfectly fine. |
| `stale` | stale | Your notes, or something above this page, changed since the last enhance. |

When you accept, Wobu notes which pages it was based on. If any of those change later, this one
turns **stale** and picks up a quiet dot in the list. Nothing is redone on its own — redoing it
spends your money and might wipe out an edit you made deliberately, so it stays your call.

## Do it from the top down

Art Style first, then World Canon, then species, cultures and places, and your characters last.
Because each enhance reads the descriptions above it, an un-enhanced parent means its children are
being written against nothing — and you will see it in the pictures.

> **Editing the machine's column** The right column is yours to edit too, field by field, including
> the colour swatches and the lists. Accepting an enhance counts as an ordinary edit as far as the
> rest of the app is concerned, so undo takes it back. The one-way rule only runs the other way:
> Wobu never touches the left column.
