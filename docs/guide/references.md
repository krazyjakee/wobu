# References and assets

Pictures are part of the description, not decoration. Every reference you add has a **job**, and
that job decides whether it steers the style, holds the shape, sets the colours — or is only ever
seen by you.

## Adding references

The **References** tab on any page is a grid, headed *Reference board*. Four ways in, all of which
attach to the page you are looking at:

- **Add images…** opens a file picker (PNG, JPEG, GIF or WebP).
- **Drag files in** from your file manager.
- **Paste** from the clipboard.
- **Pin a picture you made** from the Concepts tab.

Pictures are filed by their contents rather than their filename, inside the world folder, so two
people adding the same reference end up with the same single file and nothing clashes. Add one you
already have and it says `Already present · attached` instead of making a second copy. Dropping a
big pile in brings them one at a time on purpose: each one is a careful write to the same file, and
doing them all at once would turn a perfectly good forty-file drop into a mess of Wobu's own making.

Each tile then has a job, a strength from 0 to 1, arrows to reorder it, a mute switch, **Set cover**
and **Remove**.

## Jobs

The job is the important part — it decides what happens to the picture when you generate. Anything
you add starts as **Full reference**.

| Job | Used as | Use it for |
| --- | --- | --- |
| Silhouette | The shape to hold | Body plan and outline — the shape you want kept. |
| Palette | The colours | A swatch, or a painting whose colours you want. |
| Material | The style | Surface, weave, glaze, rust, wear. |
| Mood | Nothing — your eyes only | Atmosphere you want in your head, not in the model's. |
| Pose | The shape to hold | A stance or gesture to copy. Needs a service that takes shape references. |
| Costume | The style | Cut, layering, ornament. |
| Full reference | "This is what it looks like" | What a picture you pinned becomes. |

One picture can do two jobs on the same page — the same painting as both a palette and a full
reference — because a reference is the picture *and* the job together. A job already taken on that
page is shown as `taken` rather than hidden.

> **Mood pictures never leave your computer** A *mood* reference is never sent anywhere, and there
> is a test in Wobu making sure it stays that way. It is there so you can keep a Bruegel and a photo
> of a rained-on foundry pinned to a page without them bleeding into every picture. Not everything
> on a mood board is meant to be copied.

## Where references sit

References are inherited exactly like words are. A fabric swatch on a *Culture* page reaches every
character in that culture. A silhouette on a *Species* reaches every one of them. Put a picture at
the level where it is true, for the same reasons as [your notes](world-model.md).

### There is less room for pictures than for words

Services cap how many reference pictures they will take, and they cap them by *type*. Wobu's three
types are **objects**, **characters** and **style**, and a five-layer stack can easily offer more
style references than a model will accept. So pictures are counted per type against whatever the
chosen service allows, strongest first, and the strip across the top of the panel says where you are:

```
3/3 style refs · 2 dropped
```

Quietly binning a picture you deliberately attached would be the worst thing Wobu could do, so it
never does. If you need one that got dropped, make it stronger or mute a layer that is outbidding
it.

> **You always see what a service cannot do** A service that takes no shape references shows yours
> as downgraded to mood-board-only rather than pretending to use them. The ComfyUI setups that ship
> with Wobu currently take no pictures at all, so on your own machine *every* reference is reported
> as not sent — worth knowing before you spend an afternoon attaching them.

## Assets mode

Every picture in the world in one filterable grid — references you added and pictures you made,
together. Filter by sort (reference, generated, upload), by job, by page, or by the tags on the
pages a picture is attached to, and switch on **Unused** to find every picture nothing points at.

Click a tile and you get its size, its dimensions, its id, and every page using it with its job and
strength. Click the preview at the top of that panel to see the original full size; Escape, or the
close button, puts you back in the grid. From there you can **attach it to another page** — which is
the point of this screen. It is where pictures already in your world get reused, not where new ones
come in. Adding new ones happens on a page's References tab.

Deleting is only offered for pictures nothing is using, and it is permanent — the original and its
thumbnail both go. The record of a generated picture stays, showing the image as missing, because
that record is what you spent.

## How pictures are stored

Filed by a fingerprint of their contents, two folders deep, inside the world folder:

```
assets/
├── originals/a3/a3f9…c1.png     the picture you added or made
├── thumbs/a3/a3f9…c1.webp       its thumbnail
├── loras/7d/7d42…9e.safetensors a trained look
└── meshes/7b/7b21…04.glb        a 3D shape
```

Two things follow from that. **Duplicates cost nothing** — adding the same picture twice makes one
file. And **pictures can never clash on a shared drive**, because the same picture always lands in
the same place, which is what makes sharing a world folder safe. The file extension comes from what
the file actually is, not what it happened to be called.

Thumbnails live in the world folder rather than on your own machine on purpose: whoever adds a
picture first pays for making them, and everybody else on the shared drive gets them free. Grids and
list rows only ever load thumbnails; the full-size original is fetched one at a time, when you
actually open one.
