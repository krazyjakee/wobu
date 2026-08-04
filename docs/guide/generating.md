# Generating

Presets turn one description into the right *kind* of sheet. Generations are disposable by default;
pinning one is what changes the world.

## The controls

At the base of the inspector, under the compiled prompt: **Output preset**, **Aspect**, **Model**,
an **Extra shot prompt**, **Seed**, **Just one image**, the **Variant grid**, the project's spend
ceiling, and **Generate**.

The extra shot prompt is the one place to type free text at generation time — framing, action,
weather, a camera direction. It contributes at the Shot layer and is not saved to the world.

> **Aspect negotiation** The aspect list comes from the selected image provider. Wobu shows the
> ratio it will actually queue and its pixel dimensions beside the control, and repairs an
> unsupported or malformed saved value before it can reach the queue — saying which value it
> replaced. A flexible provider such as ComfyUI gets Wobu's curated, validated ratio vocabulary
> rather than accepting arbitrary text. Generate stays disabled until the negotiation has resolved.

## Output presets

A preset defines section priorities, framing text, an aspect and an image count. This is how the
same entity produces a costume plate one minute and a material study the next.

| Preset | Aspect | Images | Chosen by default for |
| --- | --- | --- | --- |
| Character sheet | 3:4 | 4 | character, creature |
| Turnaround | 1:1 | 8 | — |
| Portrait study | 4:5 | 4 | — |
| Costume plate | 3:4 | 2 | culture |
| Prop orthographic | 4:3 | 3 | prop, vehicle |
| Material study | 1:1 | 6 | Art Style, World Canon, species |
| Environment matte | 21:9 | 3 | environment, setting |
| Interior | 16:9 | 3 | — |

Presets reweight rather than replace. A material study still inherits your Art Style and World Canon
— it just promotes `materials` to the front and pushes `silhouette` down.

## Just one image

Tick **Just one image**, above the variant grid, to send one picture instead of the preset's whole
batch. The framing, the priorities and the aspect are still the preset's — only the count changes,
and the estimate beside Generate drops to match. The image uses the seed shown in the seed control,
so locking that seed reproduces exactly what came back.

The tick is unavailable while a variant grid is set, because the grid is already saying how many
pictures the batch is; it comes back when the grid goes Off. Presets with named views — Turnaround —
always send the whole sheet, since one image of eight views is not one of anything.

## Seeds

The seed control shows one of four states, and the wording is exact about what the *next* result
will use: unlocked, locked, locked-and-re-rolled, or locked-with-the-grid-varying-it. **Lock seed**
attaches a seed to the entity so re-rolls stay in family instead of producing a different character
each time; **Re-roll**, **Use locked** and **Clear lock** do what they say. Every result records
which of those cases produced it, so a concept's caption tells you whether it came from a locked
seed, an explicit re-roll, a variant cell or a replay.

## The variant grid

Sweep exactly one axis across a batch and compare: **Vary seed**, **Vary fragment weight**, **Vary
preset** or **Vary aspect**. You type the values, between two and sixteen of them, all
distinct; varying a fragment weight also asks which layer. The footer states how many outputs that
is and refuses clearly when a value is not supported by the provider. Presets with named views —
Turnaround — cannot be varied, because their views are already the axis.

## The job queue

Generations run through a queue that holds three jobs at a time across the whole application —
images, enhances, meshes, thumbnails and LoRA training all share it. The status bar shows the queue
depth, the active model and the last generation time, and clicking the queue jumps to Forge.

While an image runs, its tile shows a progress bar, the provider's own status note, and a live
preview on providers that stream one. **Cancel** stops it; a stalled provider or a slow NAS must
never present as a frozen app.

A failure that cost nothing is retried automatically with a widening backoff. A failure that cost
money is **not** retried behind your back: it is held, and it goes to the notification centre with
what it cost, what the provider said, and where to go next. Cancelling is not a failure and is never
reported as one.

## The Concepts tab

Results land in **Concepts** on the entity they were generated for. A tile shows the model, the seed
and where the seed came from; hovering shows the prompt. Opening one shows the whole receipt: the
recorded request, the exact recorded stack, whether the world has drifted since, and **Replay
snapshot**.

### Pinning

Pinning promotes a generated image to a reference on that entity — **Full reference** by default,
though you can pin it as a palette, a silhouette or any other role. From then on it feeds back as
conditioning for the next generation and, because references inherit, for everything downstream of
that entity too. Wobu prints the consequence under the button before you press it, naming how many
downstream entities will inherit it.

> **This is the flywheel** Iterate cheaply, promote deliberately. Generations are disposable;
> pinning is a decision. Pin a good Vashk and every Vashk character afterwards starts from a
> stronger place. It is the main tool for locking a look, and it is why the hundredth character is
> easier than the first.

### Deleting

**Delete concept…** removes the result from Concepts and the asset library and deletes its images.
Anything you pinned as a reference, or set as a cover, is kept: deleting a result you did not want
is not a decision to undo one you did. The receipt is archived rather than erased, so what the
generation cost still counts towards project spend.

## Forge mode

Forge takes the inspector's controls and promotes them to full width, with a large result grid. It
is for when you are iterating hard on one subject and want to compare twenty variants at once rather
than squinting at a narrow column.

- **Compare** up to four completed results side by side at full resolution.
- **Compose a multi-entity scene** — a primary subject plus one to three participants, in prompt
  order, with a scene direction and its own aspect. Art Style and World Canon shape a scene but
  cannot be participants in one.
- **Train an entity LoRA** from the full references you have pinned, when the selected provider
  supports it. The card states how many valid references it has of how many it needs before the
  button will do anything.

## Cost and consent

Bring-your-own-key means every call is billed to you, so Wobu is built not to surprise you:

- The Generate button shows an **estimated cost** for the batch on paid providers, and the panel
  shows what has already been spent and what is still set aside.
- A local ComfyUI provider shows **no cost at all**. That asymmetry is the point.
- A **shared spend ceiling**, recorded in the project so it applies to everyone who opens it, with a
  hard stop. A turnaround loop is exactly the kind of thing that runs two hundred images unattended.
- Every generation record stores its provider, model and parameters — so actual spend is
  reconstructable from the project folder itself, without a vendor dashboard.

The estimate is an indicative output price. Input tokens and any optional search charges are not in
it, and the panel says so.

> **Watermarking** Images generated through Google's models carry SynthID watermarking. Worth
> knowing, since this is concept art headed into a production pipeline.

## Generation records

Each generation writes one JSON file into the project folder, named by ULID and never modified:

```
generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json
```

It holds the user prompt, the compiled prompt, the negative prompt, the model, parameters and seed,
the output asset ids, and the influence snapshot. Because records are write-once and ULID-named, two
people generating at the same time on a shared folder can never collide — they simply both appear.
