# Your first project

Fifteen minutes, from an empty folder to a world that makes pictures which actually look like each
other. The order below matters: broad strokes before detail, look before subject.

## Before anything else

The first time you run Wobu it shows you two documents — the terms of use and the privacy policy —
read from the files that came with the app. Agreeing to them is the one thing you cannot skip. The
buttons are **Quit without agreeing** and **I agree — continue**.

Then there is a short tour: what the four screens are, opening or making a world, your keys, and
your first picture. **Skip for now** ends it whenever you like, and it never nags you again unless
you ask for it from the opening screen's **Show the introduction** button or Settings →
Introduction.

## Making a world

Wobu opens on the **launcher** — your recent worlds as cards, plus three buttons: **Accept
ticket…**, **Open folder…** and **New project**. There is no library to manage and nothing to sign
in to. A world is simply a folder you picked.

1. **Choose where it goes.** Anywhere: your disk, a network drive, a synced folder. If other people
   will be working on this world too, put it on the shared drive now rather than moving it later.
2. **Give it a name.** Wobu makes a self-contained folder there — notes, pictures, records of what
   you made, all inside it.
3. **Wait for the first open.** A brand new world opens instantly. An existing one has to be read
   through once so search works, and it shows you `Reading n of m files` with a **Cancel** button.
   After that it only re-reads what changed.

> **Where your stuff lives** Everything that matters is in the world folder. The search index is
> not — it sits in Wobu's own application folder, because that sort of file gets corrupted on
> network drives. Delete it and you lose nothing; the next open just takes a bit longer. Full
> layout in the [reference](reference.md).

## Write the Art Style page first

Two pages exist in every world from the moment you make it, pinned to the top of the list: **Art
Style** and **World Canon**. They are pinned there because everything you ever make is built on
them, and you should never have to go looking.

Open **Art Style** and write, in the notes column on the left, how your world should *look* — not
what is in it. The medium, the brushwork, the lighting, the colours you allow, and what should never
happen:

```
painterly, heavy brush texture, visible edges
gouache feel not airbrush
strong single key light, deep shadow, rim light on everything
palette stays muted — ash greys, ember orange, one teal accent
never: neon, chrome, lens flare, symmetrical hero poses
```

This page touches every single picture you will ever make here. Fifteen minutes spent on it is
worth more than an hour spent anywhere else.

### Then World Canon

Same idea, different question. Not how it looks — what is *true*. The era, the mood, how advanced
things are, whether there is magic, what materials exist, and what must never turn up:

```
late iron age, post-eruption, forty years after the ash fall
tone: exhausted, resourceful, not grimdark
tech: forge metal, ceramic, rope, sail. no gunpowder.
everything is scavenged, repaired, mismatched
never: firearms, printed cloth, glass windows
```

## Broad strokes before detail

The temptation is to leap straight to your favourite character. Hold off for ten more minutes and
add the layers *above* them first — a character with nothing above them is just a prompt with extra
clicking.

1. **One species.** Shape and build: the things every one of them shares. **New entity** at the
   bottom of the list; pick *Species* and name it.
2. **One culture.** Clothing, jewellery, symbols, the look of their weapons. Join it to the species
   it belongs to from the **Relations** tab.
3. **One place.** Weather, buildings, light, wear and tear. Places sit inside places — region, city,
   district — so start at whatever size you actually know something about.
4. **Now your character.** Give them a species, a culture and a home, and watch the *inherits* line
   appear under their name.

By now the panel on the right shows a stack of five or six layers, and the prompt underneath it is
already several hundred words you never had to type.

## Enhance, then generate

1. **Add a key.** Settings → Providers and models. Enhance needs a text service; Generate needs an
   image one. They can be different companies — see [Providers and keys](providers.md).
2. **Enhance each page, working downwards.** Your rough notes come back as a tidy description —
   *Silhouette, Anatomy, Materials, Palette, Signature details, Never* — appearing in the right-hand
   column as it is written. **Nothing is saved until you accept it**: you get a section-by-section
   comparison with **Keep current** and **Use new**, then **Accept selected** or **Accept all**. Do
   Art Style first, then World Canon, then the species, then down the tree.
3. **Read the prompt** in the panel on the right. Turn on **Show sources** and every phrase is
   tinted by the page that produced it. If something reads wrong, you now know exactly where to go
   and fix it.
4. **Generate.** Pick what sort of sheet you want, pick a shape, glance at the pixel size shown
   beside it, and press **Generate**. Results land on the **Concepts** tab of whoever you generated
   for.
5. **Pin the one that is right.** Pinning turns a result into a reference picture on that page —
   which then feeds into everything below it. That is the flywheel. Your world gets more consistent
   every time you use it.

> **About money** Everything is billed to your own account, so the Generate button shows an estimate
> before a paid batch runs, and a world can carry a spending limit that stops it dead. Making
> pictures on your own machine with ComfyUI shows no cost at all — that difference is deliberate.

## Habits that pay off

| | |
| --- | --- |
| Write things at the right level | If it is true of every member of a species, it belongs on the species — not repeated on nine characters. Saying things twice is exactly what Wobu is here to stop. |
| Keep your notes messy | They are raw material, not the finished thing. Fragments and contradictions are fine — Enhance is what tidies them, and it keeps your original. |
| Use *Never* lists | They become the "do not draw this" half of the prompt. "No modern firearms" written once at world level saves you forty corrections later. |
| Re-enhance when things go stale | Change a page and everything below it quietly picks up a dot. Nothing is redone behind your back — you choose when. |
| Pin deliberately | Pictures are throwaway by default. Pinning is the thing that changes your world, and it should feel like a decision. |
| Star what you are working on | The star on a row puts it in **Favourites** at the top of the list. It is just for you, on this computer, and changes nothing in the folder. |
