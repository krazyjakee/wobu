# Concept 3D

Eight views of one subject, reviewed, then reconstructed into a mesh you can turn around in the app
and hand to a modeller. The 3D tab both **makes** meshes and displays them.

## The pipeline

1. **Generate a turnaround.** Pick the Turnaround preset on a character, creature or prop. It emits
   eight views on a locked seed. The 3D tab will also start one for you — the panel is headed *Make
   a mesh*, and its empty state offers **Generate turnaround**.
2. **Review the sheet.** The tab shows the turnaround as eight slots. A view that came out
   inconsistent can be re-rolled on its own seed, before you spend anything on reconstruction.
   Re-roll as often as you like: every attempt is kept, and each slot has a `take 2/3` control for
   going back to an earlier one. Views that were never rendered are listed, with **Generate the
   missing views**.
3. **Choose the options.** **Faces** (3,000 to 1,500,000, default 500,000), a reconstruction
   **Mode**, and **PBR materials** where the backend offers them. A line beneath states exactly what
   is about to be sent: the model, how many views, and which ones.
4. **Reconstruct.** Press **Reconstruct mesh**. On a paid backend you first tick a consent box that
   says the provider charges for every submitted job, including one cancelled while it runs, and
   does not report the amount back. **Stop** cancels it like any other job.
5. **Inspect and export.** The finished mesh appears in the viewer beside the sheet — an inline
   three.js turntable with a **Wireframe** toggle and an orange one-metre marker — with **Reveal
   GLB** and **Export copy…** in the same toolbar.

> **The sheet is the last cheap place to fix a mesh** Reconstruction is one paid job and takes
> minutes. A bad back view is seconds to re-roll and cannot be repaired afterwards, so Wobu will not
> let you start until every view the selected backend needs is on screen and chosen. The front view
> is always required: a single-image reconstruction *is* the front view, and a multi-view one sends
> it first.

## Why the Turnaround preset looks the way it does

Turnaround is not a generic "spin the character" preset. It exists to feed a specific 3D backend
whose headline capability is eight-view reconstruction, so it emits exactly the view types that
backend names, in the order it wants them:

```
front · left · right · back · top · bottom · left_front · right_front
```

Each view is one generation with a locked seed and a framing fragment appended, tagged with its view
type — so the mesh stage can hand them straight over with no intermediate mapping. That is a much
stronger pipeline than single-image-to-3D, and it falls out of the influence engine for free.

The mesh backend accepts PNG and JPEG only, wants a minimum side of 128px and a maximum of 5000px,
and caps the whole batch at 6 MB before encoding. It also wants a plain background, a single object,
and the subject filling more than half the frame — which is what the preset's framing fragments are
for. Those size limits are checked when you press **Reconstruct mesh**, not when the images are
generated, so a sheet that is somehow too large to send tells you at that point.

## Text does not reach this stage

The 3.1 mesh model has no text-and-image conditioning path — image-to-3D and text-to-3D are mutually
exclusive. So the compiled prompt does **not** ride along to the mesh stage, and the mesh receipt
records an empty prompt on purpose.

In practice this costs nothing, because by the time you reach 3D all of your influence has already
been baked into the turnaround images themselves. But it does mean the mesh is only ever as good as
the views: **fix the turnaround before you reconstruct**. An inconsistent back view produces a bad
mesh, and no amount of prompt work at this stage will help.

## Jobs and waiting

The backend protocol is asynchronous — submit, then poll — and a reconstruction is a job in the same
queue as everything else, with the same Stop button and the same progress line. That queue runs
three jobs at a time in total, across every kind of work; there is no separate mesh limit.

A mesh job is minutes long, which means most of the ways it can fail happen after the money was
spent. When the provider admits to having billed, Wobu writes a receipt anyway, marked as failed and
recording **how many jobs were billed** — a count, not an amount, because the amount is not
something the provider reports.

Provider results expire within a day, so Wobu downloads the finished mesh into the project folder
rather than keeping a URL that would rot.

## What a reconstruction costs

The hosted backend charges per submitted job — including one you cancel while it is running — and
its international API does not report the amount back. There is therefore no honest figure Wobu can
put in front of you and nothing for the image spend ceiling to reserve, so paid reconstruction is
gated on that consent tick instead.

## Storage

```
assets/meshes/7b/7b21…04.glb
```

Content-addressed like every other asset, so meshes dedupe and never conflict on a shared folder.
They are also **lazy**: a `.glb` is only pulled off the share and cached locally when you actually
open the 3D tab, because a mesh is orders of magnitude larger than a thumbnail and most sessions
never need one.

## Local versus hosted

You pick the 3D backend in Settings, per project. Local ComfyUI mesh generation is an explicit
alternative to the hosted service, attractive for cost and privacy, and it carries a real quality
trade.

| | Tencent Hunyuan3D | Local 2.1 (ComfyUI) |
| --- | --- | --- |
| Views it reconstructs from | 8 on the 3.1 model, 4 on 3.0 | 1 — the front view |
| Modes | Normal, Geometry | Geometry only |
| PBR materials | yes | no |
| Cost | per submitted job, amount not reported | none |
| Setup | SecretId/SecretKey pair, a processing region, service activated | a ComfyUI install with the Hunyuan3D wrapper nodes and its weights, and at least 10 GB of VRAM |

> **Local is a different tier, not a fallback** Open weights stop at an earlier version than the
> hosted model, and that version reconstructs from a *single* image rather than eight. Wobu presents
> it as a separate quality tier and will never silently fall back to it when a key is missing. When
> the selected backend takes fewer views than a turnaround has, the panel says which ones it will
> actually send and marks the rest `not sent to this backend` rather than hiding them.
>
> Wobu does not install ComfyUI's weights or custom nodes for you, and the model licence for the
> local weights excludes the EU, the UK and South Korea — check that you are permitted to use it
> where you are.

## What you get out

A blockout mesh, exported as **GLB** and nothing else — a self-contained binary glTF is the one
format Wobu will store, so what you hand to a modeller is what the viewer showed you. Optional PBR
materials on the hosted tier, at a face count you chose.

These are concept geometry: enough to judge proportion, silhouette and scale in three dimensions,
enough to hand to a modeller as a reference, and not enough to ship.
