# Your first project

Fifteen minutes from an empty folder to a world tree that produces consistent images. The order
below is deliberate: breadth before depth, style before subject.

## Before anything else

On first run Wobu shows two documents — the terms of use and the privacy policy — read from the same
files the installer put beside the application. Agreeing to them is the one thing in Wobu you cannot
skip; the buttons are **Quit without agreeing** and **I agree — continue**.

After that comes a short introduction: what the four modes are, opening or creating a project, your
keys, and your first concept. **Skip for now** ends it at any point, and it never asks again unless
you run it from the launcher's **Show the introduction** button or from Settings → Introduction.

## Creating a project

Wobu opens on the **launcher** — recent projects as cards, plus three actions: **Accept ticket…**,
**Open folder…** and **New project**. There is no global library and no sign-in; a project is just a
directory you choose.

1. **Pick a location.** Anywhere you like — a local disk, a NAS, a synced folder. If several people
   will work on this world, put it on the share now rather than moving it later.
2. **Name it.** Wobu creates a self-contained folder at that location: notes, images, generation
   receipts, all of it inside.
3. **Wait for the first open.** A brand new project opens instantly. Opening an existing world for
   the first time reads every file to build the local search index, showing `Reading n of m files`
   and a **Cancel** button. Subsequent opens re-read only what changed.

> **Where things live** Everything canonical is in the project folder. The search index is *not* —
> it lives in your local application data, keyed by project ID, because SQLite over SMB or NFS
> corrupts. Delete the index and nothing is lost; the next open just takes a little longer. Full
> layout in the [reference](reference.md).

## Write the Art Style node first

Two nodes exist in every project from the moment it is created, pinned to the top of the navigator
above the rule: **Art Style** and **World Canon**. They are pinned because they are the roots of
every influence stack and you should never have to hunt for them.

Open **Art Style** and write, in the raw notes column, how your world should *look* — not what is in
it. Medium, rendering, line quality, lighting, palette discipline, and what to never do:

```
painterly, heavy brush texture, visible edges
gouache feel not airbrush
strong single key light, deep shadow, rim light on everything
palette stays muted — ash greys, ember orange, one teal accent
never: neon, chrome, lens flare, symmetrical hero poses
```

This is the layer that touches every single image you will ever make in this project. Fifteen
minutes here is worth more than an hour anywhere else.

### Then World Canon

Same idea, different axis: era, tone, technology and magic level, the materials that exist, and the
things that must never appear. Where Art Style governs rendering, World Canon governs *fact*.

```
late iron age, post-eruption, forty years after the ash fall
tone: exhausted, resourceful, not grimdark
tech: forge metal, ceramic, rope, sail. no gunpowder.
everything is scavenged, repaired, mismatched
never: firearms, printed cloth, glass windows
```

## Build breadth before depth

The instinct is to jump straight to your favourite character. Resist it for ten more minutes. Add
the layers *above* that character first, because a character node with nothing above it is just a
prompt with extra steps.

1. **One species.** Silhouette and anatomy — the things every member shares. **New entity** at the
   bottom of the navigator opens the sheet; pick *Species* and name it.
2. **One culture.** Costume, ornament, iconography, weapon language. Link it to the species it
   belongs to from the **Relations** tab.
3. **One setting.** Climate, architecture, ambient light, wear. Settings nest — region, city,
   district — so start at whatever scale you actually know.
4. **Now your character.** Give it a species, a culture and a home, and watch the *inherits* line
   appear under the name.

By this point the inspector on the right shows a stack of five or six layers, and the compiled
prompt beneath it is already several hundred words you never had to type.

## Enhance, then generate

1. **Add a key.** Settings → Providers and models. Enhance needs a text provider; Generate needs an
   image provider. They can be different vendors — see [Providers and keys](providers.md).
2. **Enhance each node** from the top down. Your rough notes become a structured description —
   *Silhouette, Anatomy, Materials, Palette, Signature details, Never* — streaming into the
   right-hand column. **Nothing is written until you accept it**: you get a section-by-section
   review with **Keep current** and **Use new**, then **Accept selected** or **Accept all**. Enhance
   the Art Style node first, then World Canon, then the species, then down the tree.
3. **Check the compiled prompt** in the inspector. Turn on **Show sources** and every fragment is
   tinted by the layer that produced it. If something reads wrong, you now know exactly which node
   to go and fix.
4. **Generate.** Pick an output preset and one of the aspects the selected backend offers, check the
   negotiated pixel dimensions beside the control, and press **Generate**. Results land on the
   **Concepts** tab of the node you generated for.
5. **Pin the one that is right.** Pinning promotes a generation to a reference image on that node —
   which then influences everything downstream of it. That is the flywheel; the world gets more
   consistent every time you use it.

> **A note on cost** Everything is billed to your own provider account, so the Generate button shows
> an estimated cost before a paid batch runs, and a project can carry a spend ceiling with a hard
> stop. A local ComfyUI backend shows no cost at all — that asymmetry is intentional.

## Habits that pay off

| | |
| --- | --- |
| Write at the right altitude | If a fact is true of every member of a species, it belongs on the species, not repeated on nine characters. Repetition is the thing Wobu exists to delete. |
| Keep raw notes messy | They are input, not output. Fragments and contradictions are fine — Enhance is what makes them presentable, and it keeps your original. |
| Use *Never* lists | They compile into the negative prompt. "No modern firearms" at world level saves you forty corrections later. |
| Re-enhance when things go stale | Change a node's notes and its descendants pick up a quiet stale dot. Nothing regenerates behind your back — you decide when to refresh. |
| Promote deliberately | Generations are disposable by default. Pinning is the act that changes the world, and it should feel like a decision. |
| Star what you are working on | The star on a navigator row puts it in **Favourites** at the top of the tree. It is per-machine and changes nothing on disk. |
