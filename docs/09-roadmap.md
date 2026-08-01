# 09 — Roadmap

Ordered so that each milestone is independently useful. Nothing here is dated.

### M0 — Layout prototype ✅
Static HTML/CSS/JS in [`/prototype`](../prototype) proving the three-pane workspace, the
Notes/Description split, and the Influence Stack with live prompt attribution. No Tauri.

### M1 — Shell and the world tree
Tauri 2 + React scaffold. Project create/open, the folder format from
[02](02-data-model.md), Markdown IO, SQLite index, the navigator tree with nesting and
drag-to-reparent, and the node editor with raw notes. **No AI yet.** At the end of M1 Wobu is
a usable, offline, structured world-notes app — which is the honest foundation.

### M2 — Shareable projects
Atomic guarded writes, session heartbeats and presence, conflict detection with
`.conflict-*.md` siblings, network-mount detection and the polling watcher, read-only-share
handling ([07](07-file-shares.md)).

Deliberately **before** any AI. Multi-user file corruption is the one class of bug that
destroys trust permanently, and retrofitting safe writes onto a codebase that assumed a single
local user is far harder than starting there. Every milestone after this gets the write path
for free.

### M3 — References
Image import via drag/paste, content-addressed hashing, thumbnails, the reference grid,
per-image role and weight, and the Board canvas. Assets become real context, still with no
generation.

### M4 — Enhance (first BYOK providers)
`wobu-llm`, keychain key storage, schema-constrained structured descriptions, streaming into
the editor, `stale` tracking, diff-and-accept on re-enhance. Ship **Anthropic and Gemini
together** rather than one and then the other — two providers from day one is what forces the
adapter boundary to be real, instead of one vendor's request shape wearing a trait.

### M5 — Influence Engine + first images
`wobu-influence` (resolution, fragments, text *and* per-role image budgets, attribution), the
Inspector panel, the ComfyUI adapter plus Gemini image, output presets for
character/prop/environment, capability negotiation, cost estimation and the spend ceiling,
the job queue with live previews, and the Concepts grid. **This is the first complete loop**
and the point at which Wobu is worth using.

### M6 — Iteration and consistency
Forge mode, variant grids, seed locking, pin-to-reference promotion, generation history with
replayable snapshots, and cancellable local per-entity LoRA training with project-owned weights.

### M7 — Concept 3D
Turnaround preset → image-to-3D via Hunyuan3D — hosted BYOK and/or local weights under
ComfyUI, per [08](08-providers.md) — GLB storage in the project folder, in-app three.js
viewer with turntable, export for a modeller. Concept 3D is deliberately last: it depends on
consistent multi-view output, which depends on everything above it working well.

### Later, if earned
Relationship graph view · collaborative/remote projects ·
export to a static world wiki.
