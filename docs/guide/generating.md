# Generating

Presets turn one description into the right *sort* of picture. What comes out is throwaway by
default; pinning one is the thing that changes your world.

## The controls

At the bottom of the right-hand panel, under the prompt: **Output preset**, **Aspect**, **Model**,
an **Extra shot prompt**, **Seed**, the **Variant grid**, and **Generate**.

The extra shot prompt is the one place to type something off the cuff — framing, an action, the
weather, a camera angle. It counts as part of the shot and is not saved into your world.

> **About shapes** The list of shapes comes from whichever image service you chose. Wobu shows the
> one it will actually ask for and its size in pixels right beside the control, and quietly fixes a
> saved shape the service will not accept — telling you what it changed. A flexible service like
> ComfyUI gets Wobu's own tidy list rather than being handed anything at all. Generate stays
> switched off until that is sorted out.

## Output presets

A preset decides which parts of the description to lean on, how the shot is framed, its shape and
how many pictures you get. It is how the same page gives you a costume plate one minute and a
close study of materials the next.

| Preset | Aspect | Images | Chosen by default for |
| --- | --- | --- | --- |
| Single image | 1:1 | 1 | — |
| Character sheet | 3:4 | 4 | character, creature |
| Turnaround | 1:1 | 8 | — |
| Portrait study | 4:5 | 4 | — |
| Costume plate | 3:4 | 2 | culture |
| Prop orthographic | 4:3 | 3 | prop, vehicle |
| Material study | 1:1 | 6 | Art Style, World Canon, species |
| Environment matte | 21:9 | 3 | environment, setting |
| Interior | 16:9 | 3 | — |

Presets shuffle the order rather than replacing anything. A material study still gets your art style
and your world — it just brings materials to the front and pushes silhouette back.

**Single image** is the odd one out: it shuffles nothing, and it is how you ask for one picture
rather than a whole sheet. It is offered for everything, it takes your world exactly as written, and
it makes one image — so the estimate beside Generate is the price of one. Every other preset makes
the batch in the table above; how many you get is part of choosing the preset, not a separate
setting.

## Seeds

A seed is the number that decides the random part of a picture. The control shows one of four
states, and it is precise about what the *next* picture will use: unlocked, locked, locked but
re-rolled, or locked with the grid varying it. **Lock seed** ties a seed to the page, so trying
again gives you the same character rather than a different one; **Re-roll**, **Use locked** and
**Clear lock** do what they say. Every picture records which of those it came from, so its caption
tells you whether it was a locked seed, a deliberate re-roll, one cell of a grid, or a rerun.

## The variant grid

Change exactly one thing across a batch and compare: **Vary seed**, **Vary fragment weight**, **Vary
preset** or **Vary aspect**. You type in the values, between two and sixteen of them, all different;
varying a strength also asks which layer. The footer tells you how many pictures that comes to, and
says plainly when the service will not do one of them. Presets with named views — Turnaround —
cannot be varied, because their views are already the thing that varies.

## Waiting your turn

Everything Wobu asks a service for goes through one queue, three at a time across the whole app —
pictures, enhances, 3D shapes, thumbnails and training all share it. The bottom bar shows how many
are waiting, which model is in use and how long the last one took; click the queue to jump to Forge.

While a picture is being made, its tile shows a progress bar, whatever the service is saying, and a
live preview if it offers one. **Cancel** stops it. A service that has stalled, or a slow network
drive, must never look like a frozen app.

A failure that cost nothing is retried automatically, waiting a little longer each time. A failure
that cost money is **not** retried behind your back — it is held, and sent to notifications with
what it cost, what the service said, and where to go next. Cancelling something yourself is not a
failure and is never reported as one.

## The Concepts tab

Results land in **Concepts**, on the page you made them for. A tile shows the model, the seed and
where that seed came from; hover it for the prompt. Open one and you get the whole record: what was
asked for, exactly what went into it, whether your world has moved on since, and **Replay
snapshot**.

### Pinning

Pinning turns a picture you made into a reference on that page — a **Full reference** unless you
choose otherwise. From then on it feeds into the next picture and, because references are inherited,
into everything below that page too. Wobu tells you the consequence before you press it, naming how
many pages will inherit it.

> **This is the flywheel** Try things cheaply, pin them deliberately. Pictures are throwaway;
> pinning is a decision. Pin a good Vashk and every Vashk character after it starts from a better
> place. It is the main way to lock in a look, and it is why your hundredth character is easier than
> your first.

### Deleting

**Delete concept…** removes a result from Concepts and from your pictures, and deletes the images.
Anything you had pinned as a reference, or set as a cover, stays: throwing away a result you did not
want is not the same as undoing one you did. The record of it is kept rather than erased, so what it
cost still counts towards what the world has cost you.

## Forge mode

Forge takes those same controls and gives them the whole window, with a big grid of results. It is
for when you are hammering away at one subject and want to compare twenty attempts at once rather
than squinting at a narrow column.

- **Compare** up to four finished pictures side by side, full size.
- **Put several characters in one scene** — a main subject plus one to three others, in order, with
  a line of direction and its own shape. Art Style and World Canon shape a scene but cannot be in
  one.
- **Teach a model one character's look** from the full references you have pinned, if the service
  you chose supports it. The card tells you how many usable references it has, of how many it needs,
  before the button will do anything.

## What it costs

Everything runs on your own account and is billed there. Wobu does not meter it, cap it, or
guess at what you have left — your provider's own dashboard is the only place that knows.

- Making pictures on your own machine with ComfyUI costs nothing at a provider. Paid services are
  marked as such before you generate.
- Every picture is saved with the service, model and settings that made it — so what a world cost
  can be worked out from the folder itself, alongside your provider's billing page.
- If a job is charged for and produces nothing, Wobu says so rather than quietly calling it free.

A turnaround loop is exactly the sort of thing that quietly runs two hundred pictures while you are
making tea, so set a budget alert with your provider if that matters to you.

> **A note on Google's watermarks** Pictures made through Google's models carry an invisible SynthID
> watermark. Worth knowing, since this is concept art heading into real work.

## What gets saved

Each picture writes one small file into the world folder, named by a code and never changed
afterwards:

```
generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json
```

It holds what you asked for, the prompt that was sent, the "never draw this" list, the model, the
settings, the seed, which pictures came out, and everything that went into it. Because these are
written once and never touched again, two people generating at the same moment on a shared drive can
never clash — both simply appear.
