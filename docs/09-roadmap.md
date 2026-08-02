# 09 — Roadmap

Ordered so that each product milestone is independently useful. Nothing here is dated.

## Reading status

The linked GitHub milestone and issues are the source of truth. This page is an index, not a second
tracker: when an issue reopens or its scope changes, its issue state wins until this summary is
updated.

- **Planned** — retained scope with an open implementation issue.
- **Partial** — useful pieces ship, but an open issue still blocks the documented end-to-end flow.
- **Implemented** — the canonical implementation issues are closed and the feature is reachable.
- **Validated** — implementation is closed and a milestone acceptance pass records evidence.

Product milestone numbering below follows GitHub, including the M3 sync milestone inserted after
the initial plan.

| Milestone | Status | Canonical evidence |
| --- | --- | --- |
| [M0 — Foundations & project hygiene](https://github.com/krazyjakee/wobu/milestone/9) | **Implemented** | CI, tests, errors, undo, packaging, and release infrastructure ship; stale-document reconciliation was completed by [#116](https://github.com/krazyjakee/wobu/issues/116). |
| [M1 — Shell and world tree](https://github.com/krazyjakee/wobu/milestone/1) | **Validated** | [M1 acceptance #15](https://github.com/krazyjakee/wobu/issues/15) and [acceptance evidence](13-acceptance-evidence.md#m1--structured-world-notes). |
| [M2 — Shareable projects](https://github.com/krazyjakee/wobu/milestone/2) | **Implemented** | Heartbeats/presence ([#16](https://github.com/krazyjakee/wobu/issues/16), [#17](https://github.com/krazyjakee/wobu/issues/17)), conflict and degraded-share handling ([#18](https://github.com/krazyjakee/wobu/issues/18) through [#21](https://github.com/krazyjakee/wobu/issues/21)), and network/guarded-write coverage ([#22](https://github.com/krazyjakee/wobu/issues/22), [#23](https://github.com/krazyjakee/wobu/issues/23)) are closed. |
| [M3 — Peer-to-peer sync](https://github.com/krazyjakee/wobu/milestone/10) | **Partial** | Endpoint through two-peer coverage is implemented by [#74](https://github.com/krazyjakee/wobu/issues/74) through [#85](https://github.com/krazyjakee/wobu/issues/85), with conflict and authorization follow-ups in [#89](https://github.com/krazyjakee/wobu/issues/89) and [#90](https://github.com/krazyjakee/wobu/issues/90). Share, Accept, and manage UI remains planned in [#109](https://github.com/krazyjakee/wobu/issues/109). |
| [M4 — References](https://github.com/krazyjakee/wobu/milestone/3) | **Implemented** | Asset storage/import ([#24](https://github.com/krazyjakee/wobu/issues/24) through [#26](https://github.com/krazyjakee/wobu/issues/26)) and References/Assets/Board ([#27](https://github.com/krazyjakee/wobu/issues/27) through [#30](https://github.com/krazyjakee/wobu/issues/30)) are closed. |
| [M5 — Enhance](https://github.com/krazyjakee/wobu/milestone/4) | **Implemented** | Keys/providers/schema/pipeline ([#31](https://github.com/krazyjakee/wobu/issues/31) through [#37](https://github.com/krazyjakee/wobu/issues/37)) and stale/edit/review behavior ([#38](https://github.com/krazyjakee/wobu/issues/38) through [#40](https://github.com/krazyjakee/wobu/issues/40)) are closed. |
| [M6 — Influence Engine + first images](https://github.com/krazyjakee/wobu/milestone/5) | **Validated** | The core loop is validated by [#57](https://github.com/krazyjakee/wobu/issues/57) and [acceptance evidence](13-acceptance-evidence.md#m6--first-generation-loop); provider-owned aspect selection and dimension previews were completed by [#111](https://github.com/krazyjakee/wobu/issues/111). |
| [M7 — Iteration and consistency](https://github.com/krazyjakee/wobu/milestone/6) | **Implemented** | [Forge #58](https://github.com/krazyjakee/wobu/issues/58), [variants/seed locking #59](https://github.com/krazyjakee/wobu/issues/59), [pinning #60](https://github.com/krazyjakee/wobu/issues/60), and [replay #61](https://github.com/krazyjakee/wobu/issues/61) are closed. |
| [M8 — Concept 3D](https://github.com/krazyjakee/wobu/milestone/7) | **Partial** | Turnaround, adapters, GLB storage, and viewer/export are implemented by [#62](https://github.com/krazyjakee/wobu/issues/62) through [#68](https://github.com/krazyjakee/wobu/issues/68). The UI can view/export existing meshes, but creation/review/reroll remains planned in [#110](https://github.com/krazyjakee/wobu/issues/110). |
| [M9 — Later, if earned](https://github.com/krazyjakee/wobu/milestone/8) | **Implemented** | Every retained extension has a closed canonical issue; see the feature table below. |

Cross-cutting code-health and performance milestones after M9 stay on GitHub. They are engineering
work queues, not additional user workflow stages, so this product roadmap does not duplicate them.

## Milestone scope

### M0 — Foundations & project hygiene

Cross-cutting foundations: CI, tests, linting, error handling, undo, packaging, release tooling, and
documentation discipline. These make every user feature supportable without pretending to be a
separate workflow.

### M1 — Shell and the world tree

Tauri 2 + React scaffold. Project create/open, the folder format from
[02](02-data-model.md), Markdown IO, SQLite index, the navigator tree with nesting and
drag-to-reparent, and the node editor with raw notes. **No AI yet.** At the end of M1 Wobu is
a usable, offline, structured world-notes app — which is the honest foundation.

### M2 — Shareable projects

Atomic guarded writes, session heartbeats and presence, conflict detection with
`.conflict-*.md` siblings, network-mount detection and the polling watcher, read-only-share
handling ([07](07-file-shares.md)).

Deliberately **before** any AI. Multi-user file corruption is the one class of bug that destroys
trust permanently, and retrofitting safe writes onto a codebase that assumed a single local user is
far harder than starting there. Every milestone after this gets the write path for free.

### M3 — Peer-to-peer sync

Ticket-based direct sync, last-agreed-hash reconciliation, blob transfer, conflicts, background
projects, status, and authorization are implemented. Until
[#109](https://github.com/krazyjakee/wobu/issues/109) closes, normal users cannot create, accept, or
manage a ticket from the app; the reachable collaboration workflow remains a shared folder.

### M4 — References

Image import via drag/paste, content-addressed hashing, thumbnails, the reference grid, per-image
role and weight, Assets mode, and the Board canvas. Assets become real context, still with no
generation.

### M5 — Enhance (first BYOK providers)

`wobu-llm`, keychain key storage, schema-constrained structured descriptions, streaming into the
editor, `stale` tracking, diff-and-accept on re-enhance. **Anthropic and Gemini ship together** so
the adapter boundary is real instead of one vendor's request shape wearing a trait.

### M6 — Influence Engine + first images

`wobu-influence` (resolution, fragments, text *and* per-role image budgets, attribution), the
Inspector panel, the ComfyUI adapter plus Gemini image, output presets for
character/prop/environment, capability negotiation, cost estimation and the spend ceiling, the job
queue with live previews, and the Concepts grid. **This is the first complete loop.**

Aspect choices come from the selected image backend. Unsupported or malformed saved values are
replaced before queueing, the UI previews the negotiated dimensions, and flexible backends use
Wobu's curated validated vocabulary ([#111](https://github.com/krazyjakee/wobu/issues/111)).

### M7 — Iteration and consistency

Forge mode, variant grids, seed locking, pin-to-reference promotion, and generation history with
replayable snapshots.

### M8 — Concept 3D

Turnaround preset → image-to-3D via Hunyuan3D — hosted BYOK and/or local weights under ComfyUI, per
[08](08-providers.md) — GLB storage in the project folder, in-app three.js viewer with turntable,
and export for a modeller.

The backend and viewer portions ship. Until [#110](https://github.com/krazyjakee/wobu/issues/110)
closes, the reachable product is view/export-only: users cannot review and reroll a turnaround or
start reconstruction from the UI.

### M9 — Later, if earned

| Feature | Status | Canonical issue |
| --- | --- | --- |
| Per-entity LoRA training | **Implemented** | [#69](https://github.com/krazyjakee/wobu/issues/69) |
| Relationship graph view | **Implemented** | [#70](https://github.com/krazyjakee/wobu/issues/70) |
| Cross-project style transfer | **Implemented** | [#71](https://github.com/krazyjakee/wobu/issues/71) |
| Multi-entity scene composition | **Implemented** | [#72](https://github.com/krazyjakee/wobu/issues/72) |
| Static world wiki export | **Implemented** | [#73](https://github.com/krazyjakee/wobu/issues/73) |
