# 04 — Influence Engine

The engine has two jobs: **Enhance** (notes → structured canon) and **Compile** (canon →
prompt). Both walk the same resolved stack.

## Resolving the stack

Given a subject node, walk outward and collect sources in this fixed order:

| # | Layer | Source | Contributes |
| --- | --- | --- | --- |
| 1 | Style | project Style Guide | medium, rendering, line quality, lighting model, global negatives |
| 2 | World | World Bible | era, tone, tech/magic level, material vocabulary |
| 3 | Ancestry | `species_of` chain, outermost first | body plan, anatomy, skin/hide, scale |
| 4 | Culture | `member_of` chain | costume, ornament, iconography, weapon language |
| 5 | Place | `located_in` chain (Region → City → District) | climate, architecture, ambient light, wear |
| 6 | Subject | the node itself | everything specific |
| 7 | Shot | UI controls | framing, pose, output preset, aspect |

Cycles are broken by first-visit-wins. Each collected source becomes a **layer card** in the
Inspector.

## Fragments

A layer does not contribute a paragraph — it contributes **fragments**, each with a source
node, a section key, a weight, and a routing target:

```
Fragment { text | asset, layer, node_id, section, weight, target }
target ∈ prompt · negative · style_ref · structure_ref · palette · moodboard_only
```

Weight is `link.weight × section_priority × user_slider`. Sections have intrinsic priority
per output preset — a *material study* boosts `materials` and drops `silhouette`; a
*turnaround* does the reverse.

## Compiling

1. Resolve stack → collect fragments.
2. Filter: drop muted layers and `moodboard_only` assets.
3. Score and sort by weight; stable within layer order.
4. **Budget** — trim to the model's usable prompt length, dropping lowest-weight fragments
   first. The Inspector reports what was dropped rather than truncating silently.

   There is a **second, tighter budget on images**, and it is the one that actually bites.
   Providers cap how many reference images they accept in vendor counting buckets: Gemini 3
   Pro Image, for example, allows 6 object references, 5 character references and 3 style
   references ([08](08-providers.md)). A five-layer stack can easily offer more style
   references than that on its own. So image fragments are budgeted **per provider bucket**
   against the backend's declared capability, highest weight first, and the Inspector shows
   `3/3 style refs · 2 dropped` on the layer that lost them. Silently discarding a reference
   the user deliberately attached is the worst thing this engine could do. This counting pass is
   separate from the backend's mechanism budget: ControlNet structure inputs and IPAdapter image
   prompts may have different caps and may cut across those provider buckets (#86).
5. Emit:
   - positive prompt, fragments joined by layer with the subject last (recency bias helps),
   - negative prompt from every layer's `never` section,
   - image conditioning: style refs → IP-Adapter/style transfer, structure refs → ControlNet,
     palette → colour conditioning,
   - params: model, aspect, steps, seed, LoRAs pinned by the Style Guide.
6. Persist the whole thing as the generation's `influence_snapshot`.

Every fragment keeps its `layer` through to the UI, which is what lets the compiled-prompt box
tint each span by origin. Attribution is not a debug feature — it is the main feedback loop
for learning to write good upstream notes.

## Output presets

Presets are what turn one description into the right *kind* of sheet. Each defines section
priorities, framing text, aspect, and image count. The image count lives here and nowhere else:
asking for a single picture is choosing the preset that emits one, not a second control beside
the picker that could disagree with the one already chosen.

| Preset | For | Emits |
| --- | --- | --- |
| Single image | any | one complete view, no section reweighting, ×1 |
| Character sheet | character, creature | full body, neutral pose, flat light, ×4 |
| Turnaround | character, creature, prop | 8 named views, consistent seed — see below |
| Portrait study | character | head & shoulders, dramatic key light, ×4 |
| Costume plate | character, culture | flat-laid garments and gear |
| Prop orthographic | prop, vehicle | three views on neutral ground, scale figure |
| Material study | any | close-up surface tiles, ×6 |
| Environment matte | environment, setting | wide establishing shot, atmospheric |
| Interior | environment | eye-level, practical lighting |

### The Turnaround preset is shaped by the 3D backend

Turnaround is not a generic "spin the character" preset — it exists to feed Hunyuan3D 3.1,
whose headline feature is 8-view reconstruction. So it emits exactly the view types that
backend names ([08](08-providers.md)):

```
front · left · right · back · top · bottom · left_front · right_front
```

Each view is a first-class `{ view_type, framing }` entry and one generation. `Preset::generations`
tags every entry and assigns the same caller-chosen seed to all eight; extraction appends that
entry's camera framing as a second Shot fragment. The mesh adapter can therefore hand the tags
straight to `MultiViewImages` with no application-side renaming step.

The preset also inherits the backend's input constraints — PNG/JPEG only, every side from 128 to
5000 pixels, and a combined payload under 6 MB pre-encode. A completed batch must pass through
the validated `Turnaround` type before it can become a turnaround mesh request. That constructor
sniffs the actual bytes and dimensions, checks the MIME label, requires the eight provider tags
once each in emission order, and measures the combined raw payload, so declaring these bounds is
not merely advisory.

Tencent's input guidance (plain background, no text, single object, subject filling >50% of frame) is
baked into the preset's framing fragments for the same reason.

### One plan behind the estimate, the batch and the scene

The live reference report in the Inspector, a batch, a variant grid and a scene composition are
one planning path with four entry points, not four pipelines. A request is normalized once —
provider and model, seed and its provenance, preset, Shot controls, sliders and aspect — and then
expanded into one *cell* per image, each carrying its own preset, aspect, seed and slider values.
Everything after that reads those cells: the compiled prompt, the reference budget and the receipt.

The report is therefore not a second opinion. It negotiates the same first cell the batch would
send first, so a grid that silences a reference in its opening cell reports that reference as
withheld rather than counting one the image would not carry. Two things about it are deliberately
different, because the panel is advisory and free: an unseeded report uses seed zero rather than
minting a random one, so nudging a slider twice gives the same answer twice; and it reads a local
ComfyUI's *declared* capabilities rather than probing the server, so a machine that is switched
off leaves the panel optimistic instead of turning it into an error. Generation itself always
probes.

Replay joins this path only at its last step, the queue. It re-sends a recorded request rather
than compiling a new one, which is what makes it a replay. Mesh reconstruction shares only the
first step, provider selection: it consumes finished generations and has no stack, preset or
aspect of its own.

### Locked seeds and variant grids

An entity may persist one shared `locked_seed` in its Markdown frontmatter. Generate uses that
seed whenever the Inspector has not explicitly re-rolled it; ordinary preset batches retain
their deterministic adjacent-seed family, and each receipt records whether it used the exact
lock, a derived member of that family, an explicit re-roll, or a seed-grid cell. A scene has no
lock of its own — its participants may each have one and may disagree — so a composition is
either explicitly re-rolled or random.

A variant grid emits one image per explicit cell value. It either varies seed while holding the
compiled inputs fixed, or holds one seed while varying exactly one of fragment weight, preset,
or aspect. Named-view presets such as Turnaround are excluded because reducing an eight-view
contract to one cell would destroy its meaning. Every receipt stores a typed `variation` object
under `Generation.params`: grid id, cell index/total, axis, and the axis-specific value. This is
enough to regroup and reconstruct the grid without parsing prompts.

Forge exposes this same Inspector state rather than maintaining a second generation form. It pairs
the controls and visible compiled prompt with a virtualized receipt grid for the chosen subject;
receipt tiles surface their variation axis, and two to four completed outputs can be compared from
their full-resolution originals without eagerly loading those originals into the grid.

### History, replay and drift

Each completed request also records the transient sliders and Shot controls that produced its
snapshot. History can therefore resolve today's world under the same controls and compare layers,
weights, fragments, and compiled prompts. Older receipts that predate those controls are still
diffable, but the UI labels that comparison as today's default controls rather than claiming every
weight difference is a world edit.

Replay is intentionally not another compilation. It sends the recorded positive and negative
prompts, backend/model, seed, negotiated aspect and resolution, and the kept snapshot references in
their recorded provider order. It works even after the subject node has been deleted because the
receipt and content-addressed assets are the source of truth. If any reference asset is missing,
replay stops explicitly; reading today's links would make a plausible new image, not a replay.

### Multi-entity scene composition

Forge can compose two to four ordered entities into one `environment_matte` request. Each entity's
stack resolves independently first. The composition then emits shared Style/World nodes once;
deduplicates ancestry, culture and place sources in layer/participant order; emits every Subject in
the order chosen; and appends one Shot. A shared source uses the strongest path weight rather than
the sum, so adding another entity from the same culture cannot accidentally double the culture or
house style.

The compiled prompt states shared world/style guidance first, then one explicitly named clause per
entity. Distinct palettes remain inside those entity clauses instead of being averaged into a colour
scheme that belongs to nobody. Shot clauses are deliberately last in this order: the ordinary wide
establishing framing, the user's scene direction, then a final instruction to preserve every named
identity. Shared exact fragments and negative terms are emitted once.

Reference images still pass through adapter-mechanism limits and the provider's declared counting
buckets. Inside each bucket, exact `(asset, role)` duplicates cost one slot and each participant's
strongest direct reference is protected before spare slots are filled by weight. If negotiation
cannot retain any offered identity reference for one participant, composition refuses before a paid
request rather than silently producing a scene with that entity visually erased.

The immutable receipt remains compatible with single-subject history: `node_id` is its primary
index anchor, while `params.sceneComposition` records version 1 plus every ordered subject id/name.
The influence snapshot is the complete merged stack. Scene metadata is separate from LoRA
application/downgrade metadata, and request preparation carries the full participant list so
compatible pins can be collected across all entities and deduplicated by weight hash.

## The Enhance pipeline

`⌘E` on any node:

1. Build an LLM context: the resolved stack's **descriptions** (not their raw notes), plus
   this node's `notes_raw`, plus its attributes and the names of its reference images' roles.
2. Ask for a structured description via tool-use, so output is schema-valid JSON rather than
   parsed prose. Sections vary by kind (see [02](02-data-model.md)).
3. Stream into the right-hand pane; the user can stop, edit, or reject.
4. On accept, set `description_state = fresh` and stamp the upstream node versions used.
   When any of those change later, the state flips to `stale`.

Constraints given to the model, which matter more than the prompt wording:

- **Do not invent facts the notes don't imply.** Elaborate on what is there; ask in a
  `questions` field for what's missing rather than confabulating.
- **Write visually.** Every sentence should change what a renderer would draw. No history,
  motives or plot unless they are visible on the body.
- **Do not restate inherited traits.** If the species already establishes four-jointed legs,
  the character description must not repeat it — that's duplication in the compiled prompt.
  Only note *deviations* from the inherited baseline.
- **Populate `never`.** Explicit negatives are how visual drift is prevented.

That third constraint is the subtle one, and it's the reason Enhance must see the whole stack
rather than just the node's own notes. Each layer stays lean and orthogonal, and the compiled
prompt stays inside budget.

## Consistency across generations

- **Seed locking** per entity, so re-rolls stay in family.
- **Pinned references**: promoting a generated image to a `full_ref` reference makes it feed
  back as conditioning for the next generation — the main tool for locking a look.
- **Per-entity LoRA**: once an entity has at least 15 distinct, enabled `full_ref` originals,
  Forge offers a cancellable local fine-tune. A fixed `wobu-lora-trainer` executable receives a
  private staged manifest—never a project-supplied command—and returns a validated safetensors
  file. Its content-addressed project pin follows the resolved influence/scene order; compatible
  pins are hash-deduplicated, their trigger tokens are added once, and ComfyUI applies the ordered
  vector automatically. Model, provider, installation or integrity mismatches remain visible
  receipt downgrades rather than silent omissions.
