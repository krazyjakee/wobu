# The influence stack

The panel on the right is the whole idea of Wobu in one column: which parts of your world are
feeding this picture, how strongly, and exactly which words each one put in.

## What goes into a picture

Starting from whoever you have selected, Wobu works outwards and gathers everything that applies —
broadest first, the subject last:

| # | Layer | Comes from | Puts in |
| --- | --- | --- | --- |
| 1 | Style | The Art Style page | Medium, rendering, line quality, lighting, things to never draw |
| 2 | World | The World Canon page | Era, mood, how advanced things are, what materials exist |
| 3 | Ancestry | The species it belongs to | Body plan, anatomy, skin or hide, size |
| 4 | Culture | The culture it belongs to | Clothing, jewellery, symbols, the look of their weapons |
| 5 | Place | Where it lives | Weather, buildings, light, wear |
| 6 | Subject | The page itself, and anything joined sideways to it | Everything particular to it |
| 7 | Shot | The controls underneath | Framing, pose, sort of sheet, shape |

Art Style and World Canon are always included, whether or not anything points at them. Whatever a
page sits inside counts fully, so a district gets its city, which gets its region. Each thing that
gets gathered up becomes one card in the panel, and the card tells you *why* it is there.

## Lines, not paragraphs

A layer does not hand over a block of prose. It hands over **fragments** — single lines. Each one
knows which page it came from, which section of that page, how strong it is, and where it should
go: into the prompt, into the "never draw this" list, in as a style reference, in as a shape
reference, in as colours, or nowhere at all except your own mood board.

How strong a fragment ends up being is three things multiplied together:

```
how strong you made the link  ×  how much this sort of sheet cares about that section  ×  your slider
```

That middle one is why the sort of sheet matters. A material study pushes `materials` forward and
`silhouette` back; a turnaround does the opposite. The same description makes a different prompt
depending on what you asked for.

## How the prompt is put together

1. **Gather.** Work outwards and collect every line.
2. **Drop what you switched off.** Muted layers go, and so does anything marked as being for your
   eyes only — those are not reported as dropped, because they were never in the running.
3. **Sort** by strength, keeping the layer order.
4. **Trim to fit.** Every model has a limit on how long a prompt can be. Wobu cuts the *weakest*
   lines first — and tells you what it cut rather than quietly chopping the end off. Pictures are
   handled separately and more strictly, against what the service says it will take.
5. **Send.** The lines go out in the order they were gathered — layer by layer, subject last, the
   framing after that. The "never draw this" list is built from every layer's *Never* section; style
   references go in as style, shape references as shape, palettes as colour.
6. **Keep a record.** The whole lot — every strength, every mute, everything dropped — is saved
   alongside the picture.

## Reading the prompt

Every line keeps track of where it came from, all the way to the screen. **Show sources** turns the
prompt from plain text into tinted phrases, one colour per layer. Hover one and it names the layer
and its strength; click it and you land on the page that wrote it.

```
style     painterly gouache, heavy brush texture, strong single key light, deep shadow
world     late iron age, forty years post-eruption, scavenged and repaired materials
ancestry  tall narrow-shouldered digitigrade humanoid, four-jointed legs, ash-grey hide
culture   ember guild kiln-glaze plate over oiled leather, rope fastenings, collarbone signet
place     cinder bay harbour light, salt-bleached timber, ash haze
subject   Kael Vantris — armour cut for a broader frame, cinched with rope; burn scar left jaw
          to temple; signet ground flat; unlit ashglass lantern at the belt
```

**This is not a debugging tool.** It is how you learn to write good notes. When a picture comes out
wrong, the colour tells you which page to go and fix — and over a few weeks it teaches you what
belongs where far better than any guide could.

Both halves — what to draw and what not to draw — show how many lines and characters they came to,
and can be copied.

### What was left out, and why

Underneath the prompt are up to two lists, worded differently on purpose:

- **Turned down** — a slider or a link is at zero. Raise it and these come straight back; nothing
  has been edited.
- **Did not fit** — these were the weakest lines when the prompt ran too long. Write less further
  up, or make these sources stronger so something else goes instead.

If everything got cut, Wobu says so rather than quietly sending an empty prompt. And if a single
line is longer than the whole allowance, it is sent long rather than not at all, and the panel tells
you by how much.

## Layer cards

Each card in the stack gives you:

| | |
| --- | --- |
| Thumbnail and layer name | Which of the seven layers this is, and what it looks like. |
| Which page | With an **Open source** button that takes you there so you can edit it. |
| Counts | `4 text · 2 images · 1 sent · 1 dropped` — what this layer actually contributed. |
| Strength slider | Scales everything from this layer, from 0 to 1. |
| Mute switch | Leaves this layer out of this picture entirely. |
| The lines themselves | Exactly what it is putting in, word for word, each with its own strength. |

> **Changes here last one picture** Muting and turning things down in this panel affect **this
> picture only** — they never edit your world. That separation is deliberate and absolute. If you
> find yourself muting the same layer every single time, that is not a workflow, it is a hint: go
> and change the page, or change the link.
>
> Link strengths are different — they *are* part of your world. Set those on the Relations tab of
> the page itself, when a character really is only loosely part of their culture.

## Steering it

| | |
| --- | --- |
| The art style is drowning the character | Drop the Style layer to about 0.6 for this shot. If it keeps happening, your Art Style notes are too specific — move some of that detail down to World or Culture. |
| The character keeps coming out generic | Look at how many lines the Subject layer contributed. A thin subject usually means the notes are backstory rather than description — Enhance drops anything you cannot see. |
| Two characters look identical | Their own descriptions are not saying enough that differs. Ancestry and culture are doing all the work, which is exactly what they are meant to do. |
| Something out of period keeps appearing | Add it to *Never* on the highest page where it is always wrong — usually World Canon. It then applies to everything below. |
| Lines keep getting dropped | The prompt is running too long. Mute a layer you do not need this time, or find the duplication — usually a page repeating what the one above it already said. |
| A reference picture seems to be doing nothing | Check its job, and check the service. *Mood* pictures are never sent anywhere, shape references do nothing on services that do not take them, and the ComfyUI setups that ship with Wobu take no pictures at all. |
| The "never draw this" list vanished | Some image models have no way to take one. Wobu holds it back and tells you, rather than pasting your *Never* list into the prompt where it would summon the very thing you banned. |

## Doing it again later

Every picture is saved with a record of exactly what went into it — every strength, every mute, at
the moment you pressed Generate. That is what lets you rerun something six months later, after the
world has moved on and three of those pages have been rewritten.

Open a result and Wobu shows you what it recorded next to how things stand today, and names the
difference: *The prompt has changed*, or *The parts that can be compared are unchanged*. **Run these
settings again** sends exactly what was recorded, without looking at today's world at all.
