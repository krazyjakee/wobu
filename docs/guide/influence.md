# The influence stack

The panel on the right is the whole product in one column: which layers are contributing to this
image, how much each one weighs, and exactly which words each one put in the prompt.

## How the stack resolves

Given the node you have selected, Wobu walks outward and collects sources in a fixed order —
outermost first, subject last:

| # | Layer | Source | Contributes |
| --- | --- | --- | --- |
| 1 | Style | The Art Style node | Medium, rendering, line quality, lighting model, global negatives |
| 2 | World | The World Canon node | Era, tone, tech and magic level, material vocabulary |
| 3 | Ancestry | the `species_of` chain | Body plan, anatomy, skin and hide, scale |
| 4 | Culture | the `member_of` chain | Costume, ornament, iconography, weapon language |
| 5 | Place | the `located_in` chain | Climate, architecture, ambient light, wear |
| 6 | Subject | the node itself, plus anything `related_to` | Everything specific |
| 7 | Shot | the controls below | Framing, pose, output preset, aspect |

Art Style and World Canon are seeded into every stack whether or not anything links to them. A
node's parent is an implicit edge of full weight, so a district inherits its city which inherits its
region. Each source collected becomes one card in the inspector, and the card says *why* it is
there.

## Fragments, not paragraphs

A layer does not contribute a block of prose — it contributes **fragments**. Each fragment knows its
source node, which description section it came from, its weight, and where it should be routed:

```
prompt · negative · style_ref · structure_ref · palette · moodboard_only
```

A fragment's weight is the product of three things:

```
link weight  ×  section priority (per preset)  ×  your slider
```

Section priority is why presets matter. A *material study* boosts `materials` and drops
`silhouette`; a *turnaround* does the reverse. The same description produces a different prompt
depending on what kind of sheet you asked for.

## Compiling

1. **Resolve and collect.** Walk the stack, gather every fragment.
2. **Filter.** Drop muted layers and anything marked mood-board-only. Mood-board fragments are not
   reported as dropped, because they were never candidates.
3. **Score and sort** by weight, stable within layer order.
4. **Budget.** Trim to the model's usable prompt length, dropping the *lowest-weight* fragments
   first — and report what was dropped rather than truncating in silence. Images are budgeted
   separately and more tightly, per bucket, against the backend's declared limits.
5. **Emit.** Fragments go out in arrival order — layer by layer, subject last, the shot's framing
   after that — rather than in the order they were dropped. The negative prompt is built from every
   layer's *Never* section; style references go to style conditioning, structure references to
   structure conditioning, palettes to colour conditioning.
6. **Snapshot.** The entire resolved stack — every weight, every mute, every dropped fragment — is
   saved with the generation.

## Reading the compiled prompt

Every fragment keeps its layer identity all the way to the screen. **Show sources** turns the
compiled prompt from prose into tinted spans, one colour per layer; hovering one names the layer and
its weight, and clicking it opens the node that produced it.

```
style     painterly gouache, heavy brush texture, strong single key light, deep shadow
world     late iron age, forty years post-eruption, scavenged and repaired materials
ancestry  tall narrow-shouldered digitigrade humanoid, four-jointed legs, ash-grey hide
culture   ember guild kiln-glaze plate over oiled leather, rope fastenings, collarbone signet
place     cinder bay harbour light, salt-bleached timber, ash haze
subject   Kael Vantris — armour cut for a broader frame, cinched with rope; burn scar left jaw
          to temple; signet ground flat; unlit ashglass lantern at the belt
```

**Attribution is not a debug feature.** It is the main feedback loop for learning to write good
upstream notes. When an image comes out wrong, the tint tells you which node to go and fix — and
over a few weeks it teaches you what belongs at which altitude far more effectively than any
documentation could.

Both channels — the prompt and the negative — show their fragment and character counts and can be
copied.

### What was left out, and why

Beneath the prompt are up to two lists, and they are worded differently on purpose:

- **Turned down** — a slider or a link weight is at zero. Raise it and these come straight back;
  nothing has been edited.
- **Did not fit** — these were the lightest fragments when the prompt ran over budget. Write leaner
  notes upstream, or weight these sources higher so something else goes first.

If everything was cut, Wobu says so plainly rather than quietly sending an empty prompt. If a single
fragment is longer than the whole budget it is sent long rather than sent blank, and the panel tells
you by how much.

## Layer cards

Each card in the stack gives you:

| | |
| --- | --- |
| Thumbnail and layer name | Which of the seven layers this is, and what the source looks like. |
| Source node | With an **Open source** button that jumps to it so you can edit it. |
| Counts | `4 text · 2 images · 1 sent · 1 dropped` — what this layer actually contributed. |
| Weight slider | Scales every fragment from this layer, from 0 to 1. |
| Mute toggle | Removes the layer from this generation entirely. |
| Fragment list | The exact text and images it contributes, verbatim, each with its own weight. |

> **Inspector edits are per-generation** Muting and reweighting here affect **this generation only**
> — they never edit the world. That separation is deliberate and absolute. If you find yourself
> muting the same layer on every render, that is not a workflow, that is a signal: go and change the
> node, or change the link.
>
> Link weights, by contrast, *are* part of the world. Set them from the node's Relations tab when a
> character should genuinely be only loosely of its culture.

## Practical steering

| | |
| --- | --- |
| Style is overpowering the subject | Drop the Style layer to about 0.6 for this shot. If it happens constantly, your Art Style notes are too specific — move detail down to World or Culture. |
| Character keeps looking generic | Check the Subject layer's fragment count. A thin subject layer usually means the notes are backstory rather than description — Enhance drops anything not visible on the body. |
| Two characters look identical | Their subject layers are not saying enough that differs. The ancestry and culture layers are doing all the work, which is exactly what they are supposed to do. |
| Something anachronistic keeps appearing | Add it to *Never* on the highest layer where it is always wrong — usually World Canon. It compiles into the negative prompt for every descendant. |
| Fragments are being dropped | You are over prompt budget. Mute a layer you do not need for this shot, or find the duplication — usually a child restating what its parent already said. |
| A reference is not being used | Check its role, and check the backend. `mood` is never sent anywhere, structure roles are inert on backends that do not accept them, and the shipped ComfyUI workflows take no images at all. |
| The negative prompt vanished | Some image models have no negative conditioning. Wobu withholds it and reports that rather than pasting your *Never* list into the positive prompt. |

## The snapshot

Every generation stores its influence snapshot — the exact resolved stack, weights and all, at the
moment you pressed Generate. That is what makes a result reproducible six months later, after the
world has moved on and three of those layers have been rewritten.

Open a result and Wobu shows the recorded stack next to today's, and names the difference: *Prompt
drifted*, or *Comparable compiled prompts are unchanged*. **Replay snapshot** resubmits the recorded
request as recorded, without reading today's world at all.
