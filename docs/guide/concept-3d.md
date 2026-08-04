# Concept 3D

Eight views of one subject, checked over, then turned into a rough 3D shape you can spin around in
the app and hand to a modeller. The 3D tab both **makes** these and shows them.

## Start to finish

1. **Make a turnaround.** Pick the Turnaround preset on a character, creature or prop. It makes
   eight views on one locked seed. The 3D tab will start one for you too — the panel is headed *Make
   a mesh*, and offers **Generate turnaround** when it is empty.
2. **Check the sheet.** The tab shows the turnaround as eight slots. A view that came out wrong can
   be redone on its own, before you spend anything on the 3D. Redo them as often as you like: every
   attempt is kept, and each slot has a `take 2/3` control for going back to an earlier one. Views
   that were never made are listed, with **Generate the missing views**.
3. **Choose your options.** **Faces** (3,000 to 1,500,000, 500,000 by default), a **Mode**, and
   **PBR materials** where the service offers them. A line underneath says exactly what is about to
   be sent: the model, how many views, and which ones.
4. **Make the mesh.** Press **Reconstruct mesh**. On a paid service you first tick a box confirming
   you understand that it charges for every job you send — including one you cancel while it is
   running — and does not tell anybody what it charged. **Stop** cancels it like anything else.
5. **Look at it, then export it.** The finished shape appears in the viewer beside the sheet — you
   can spin it with **Turntable**, switch on **Wireframe**, and judge scale against the orange
   one-metre marker. **Show the file** and **Export copy…** are in the same row of buttons.

> **The sheet is your last cheap chance to fix it** Making the 3D shape is one paid job and takes
> minutes. A bad back view takes seconds to redo and cannot be fixed afterwards, so Wobu will not
> let you start until every view the chosen service needs is on screen and picked. The front view is
> always required: a single-picture reconstruction *is* the front view, and a multi-view one sends
> it first.

## Why the Turnaround preset is the shape it is

Turnaround is not a generic "spin the character round" preset. It exists to feed one particular 3D
service, whose whole trick is building a shape from eight views — so it makes exactly the views that
service asks for, in the order it wants them:

```
front · left · right · back · top · bottom · left_front · right_front
```

Each view is one picture on the same locked seed with a bit of framing added, tagged with which view
it is — so the 3D stage can hand them straight over with nothing in between. That gives far better
results than a single picture would, and it falls out of the rest of Wobu for free.

That service takes PNG and JPEG only, wants each picture at least 128 pixels and at most 5000 on a
side, and caps the whole batch at 6 MB. It also wants a plain background, one object, and the
subject filling more than half the frame — which is what the preset's framing is for. Those limits
are checked when you press **Reconstruct mesh**, not when the pictures were made, so a sheet that is
somehow too big to send tells you then.

## Words do not reach this stage

The 3D model has no way to take a picture and a description at the same time — it is one or the
other. So your prompt does **not** travel to the 3D stage, and the record of it is deliberately
blank.

In practice that costs you nothing, because by the time you get here all of your world is already
baked into the turnaround pictures. But it does mean the shape is only ever as good as the views:
**fix the turnaround before you make the mesh**. A wonky back view makes a wonky shape, and no
amount of writing will help at this point.

## Waiting

The service works by taking your job and being asked later whether it is done, and a reconstruction
waits in the same queue as everything else, with the same Stop button and the same progress line.
That queue runs three jobs at a time across the whole app; there is no separate limit for 3D.

A 3D job takes minutes, which means most of the ways it can go wrong happen after the money has
gone. When the service admits it has charged, Wobu keeps a record anyway, marked as failed and
noting **how many jobs were billed** — a count, not an amount, because the amount is not something
it tells anyone.

Finished shapes disappear from the service within a day, so Wobu downloads yours into the world
folder rather than keeping a link that would rot.

## What it costs

The hosted service charges per job you send — including one you cancel while it runs — and its
international interface does not report the amount. So there is no honest figure Wobu could put in
front of you and nothing for the spending limit to set aside, which is why paid reconstruction is
behind that tick box instead.

## Where shapes live

```
assets/meshes/7b/7b21…04.glb
```

Filed by their contents like every other picture, so they never duplicate and never clash on a
shared drive. They are also **fetched only when needed**: a mesh is only pulled off the shared drive
when you actually open the 3D tab, because it is enormously bigger than a thumbnail and most of the
time nobody needs one.

## Your own machine, or a paid service

You pick the 3D service in Settings, per world. Making shapes locally in ComfyUI is a real
alternative — cheaper, and nothing leaves your computer — but the quality is genuinely different.

| | Tencent Hunyuan3D | Local 2.1 (ComfyUI) |
| --- | --- | --- |
| Views it can use | 8 on the 3.1 model, 4 on 3.0 | 1 — the front view |
| Modes | Normal, Geometry | Geometry only |
| PBR materials | yes | no |
| Cost | per job sent, amount never reported | none |
| Setting it up | an id and key pair, a region, and the service switched on | a ComfyUI install with the Hunyuan3D add-ons and their model files, and at least 10 GB of video memory |

> **Local is a different tier, not a backup** The freely available models stop at an earlier version
> than the paid one, and that version builds its shape from a *single* picture rather than eight.
> Wobu treats it as its own quality tier and will never quietly fall back to it because a key is
> missing. When the service you chose takes fewer views than your turnaround has, the panel says
> which ones it will actually send and marks the rest `not sent to this provider` rather than hiding
> them.
>
> Wobu does not install ComfyUI's add-ons or model files for you, and the licence on the local model
> files excludes the EU, the UK and South Korea — check you are allowed to use it where you are.

## What you end up with

A rough shape, exported as **GLB** and nothing else — one self-contained file is the only format
Wobu will keep, so what you hand to a modeller is exactly what you were looking at. Optional PBR
materials on the paid tier, at whatever level of detail you chose.

This is concept geometry: enough to judge proportion, outline and scale in three dimensions, enough
to hand to a modeller as a starting point, and nowhere near enough to ship.
