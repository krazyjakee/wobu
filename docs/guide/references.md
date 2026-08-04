# References and assets

Images are first-class context, not decoration. A reference carries a role, and the role is what
tells the compiler whether it becomes style conditioning, structure conditioning, a colour pass — or
something only you ever see.

## Adding references

The **References** tab on any entity is a grid, headed *Reference board*. Four ways in, all of which
attach to the entity you are looking at:

- **Add images…** opens a file picker (PNG, JPEG, GIF or WebP).
- **Drag files in** from your file manager.
- **Paste** from the clipboard.
- **Pin a generated image** from the Concepts tab.

Images are hashed and stored inside the project folder, so two people importing the same reference
produce the same file and nothing conflicts. An import you already have reports `Already present ·
attached` rather than making a second copy. Large drops import one at a time on purpose: every
attachment is a guarded write to the same Markdown file, and running them in parallel would turn a
successful forty-file drop into conflicts Wobu created itself.

Each tile then carries a role, a weight from 0 to 1, reorder arrows, a mute toggle, **Set cover**
and **Remove**.

## Roles

The role is the important part — it decides where the image is routed at generation time. A new
import starts as **Full reference**.

| Role | Routed to | Use it for |
| --- | --- | --- |
| Silhouette | Structure conditioning | Body plan and outline — the shape you want held. |
| Palette | Colour conditioning | A swatch or a painting whose colour relationships you want. |
| Material | Style conditioning | Surface, weave, glaze, corrosion, wear. |
| Mood | Nothing — human only | Atmosphere you want in your head, not in the model's. |
| Pose | Structure conditioning | A stance or gesture to copy. Needs a provider that accepts structure references. |
| Costume | Style conditioning | Garment cut, layering, ornament. |
| Full reference | Character or object reference | "This is what it looks like." What a pinned generation becomes. |

A single image can hold two roles on the same entity — the same painting as both a palette and a
full reference — because a reference is identified by the pair of image and role. A role already
taken on that entity is shown as `taken` rather than hidden.

> **Mood is deliberately inert** A `mood` reference is never sent to a provider, and there is a test
> in the engine asserting it is the only role that never leaves the machine. It exists so you can
> keep a Bruegel and a photo of a rained-on foundry on the entity without them leaking into every
> render. Not everything on a mood board should be conditioning.

## Where references sit in the hierarchy

References inherit exactly like text does. A material swatch on your *Culture* entity reaches every
character in that culture. A silhouette reference on a *Species* reaches every member. Put an image
at the altitude where it is true, for the same reasons as [notes](world-model.md).

### The image budget bites harder than the text budget

Providers cap how many reference images they accept, and they cap them *by bucket*. Wobu's three
buckets are **objects**, **characters** and **style refs**, and a five-layer stack can easily offer
more style references than a model will take. So images are budgeted per bucket against whatever the
selected provider declares, highest weight first, and the strip across the top of the influence
stack says so:

```
3/3 style refs · 2 dropped
```

Silently discarding a reference you deliberately attached would be the worst thing this engine could
do, so it never does. If you need a dropped reference, raise its weight or mute a layer that is
outbidding it.

> **Capability differences are visible, not hidden** A provider that accepts no structure references
> shows yours as downgraded to mood-board-only rather than pretending to use them. The shipped
> ComfyUI workflows currently take no image input at all, so on a local provider *every* reference
> is reported as not sent — which is worth knowing before you spend an afternoon attaching them.

## Assets mode

Every image in the project in one filterable grid — imported references and generated results
together. Filter by kind (reference, generated, upload), by role, by entity, or by the tags of the
entities an image is linked to, and toggle **Unused** to see every image nothing points at.

Selecting a tile shows its dimensions, size, id, and every entity that uses it with each role and
weight. Click the preview at the top of that panel to open the original full size; Escape, or the
close button, puts you back in the grid. From there you can **attach it as a reference** to another
entity — which is the point of
the mode: it is where an image already in the project gets reused, rather than where images come in.
Importing happens on an entity's References tab.

Deletion is offered only for unused images, and it is permanent: it removes the original and its
thumbnail. A generated image's receipt survives and shows the output as missing, because the receipt
is the record of what you spent.

## How images are stored

Content-addressed by hash, sharded two levels deep, inside the project folder:

```
assets/
├── originals/a3/a3f9…c1.png     the file you imported or generated
├── thumbs/a3/a3f9…c1.webp       generated thumbnail
├── loras/7d/7d42…9e.safetensors trained entity weights
└── meshes/7b/7b21…04.glb        concept 3D output
```

Two consequences worth knowing. **Deduplication is free** — importing the same image twice costs one
file. And **asset writes can never conflict**, because the same bytes always land at the same path,
which is what makes shared project folders safe. The file extension comes from what the bytes
actually are, not from what the file you dropped was called.

Thumbnails live in the project folder rather than in local cache on purpose: the first person to
import an image pays the cost of generating them and everyone else on the share gets them free.
Grids and navigator rows bind to thumbnails only; full-resolution originals are fetched one at a
time when you actually open an image.
