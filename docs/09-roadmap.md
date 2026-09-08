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
| [M3 — Peer-to-peer sync](https://github.com/krazyjakee/wobu/milestone/10) | **Implemented** | Endpoint through two-peer coverage is implemented by [#74](https://github.com/krazyjakee/wobu/issues/74) through [#85](https://github.com/krazyjakee/wobu/issues/85), with conflict and authorization follow-ups in [#89](https://github.com/krazyjakee/wobu/issues/89) and [#90](https://github.com/krazyjakee/wobu/issues/90). Share, Accept, reopen, and manage/unshare UI ships in [#109](https://github.com/krazyjakee/wobu/issues/109). |
| [M4 — References](https://github.com/krazyjakee/wobu/milestone/3) | **Implemented** | Asset storage/import ([#24](https://github.com/krazyjakee/wobu/issues/24) through [#26](https://github.com/krazyjakee/wobu/issues/26)) and References/Assets ([#27](https://github.com/krazyjakee/wobu/issues/27) through [#30](https://github.com/krazyjakee/wobu/issues/30)) are closed. The mood-board canvas shipped in that milestone and was retired by [#144](https://github.com/krazyjakee/wobu/issues/144). |
| [M5 — Enhance](https://github.com/krazyjakee/wobu/milestone/4) | **Implemented** | Keys/providers/schema/pipeline ([#31](https://github.com/krazyjakee/wobu/issues/31) through [#37](https://github.com/krazyjakee/wobu/issues/37)) and stale/edit/review behavior ([#38](https://github.com/krazyjakee/wobu/issues/38) through [#40](https://github.com/krazyjakee/wobu/issues/40)) are closed. |
| [M6 — Influence Engine + first images](https://github.com/krazyjakee/wobu/milestone/5) | **Validated** | The core loop is validated by [#57](https://github.com/krazyjakee/wobu/issues/57) and [acceptance evidence](13-acceptance-evidence.md#m6--first-generation-loop); provider-owned aspect selection and dimension previews were completed by [#111](https://github.com/krazyjakee/wobu/issues/111). |
| [M7 — Iteration and consistency](https://github.com/krazyjakee/wobu/milestone/6) | **Implemented** | [Forge #58](https://github.com/krazyjakee/wobu/issues/58), [variants/seed locking #59](https://github.com/krazyjakee/wobu/issues/59), [pinning #60](https://github.com/krazyjakee/wobu/issues/60), and [replay #61](https://github.com/krazyjakee/wobu/issues/61) are closed. |
| [M8 — Concept 3D](https://github.com/krazyjakee/wobu/milestone/7) | **Implemented** | Turnaround, adapters, GLB storage, and viewer/export are implemented by [#62](https://github.com/krazyjakee/wobu/issues/62) through [#68](https://github.com/krazyjakee/wobu/issues/68); review, per-view reroll, and queued reconstruction are joined to them by [#110](https://github.com/krazyjakee/wobu/issues/110). |
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
projects, status, authorization, and the Share/Accept/manage workflow are implemented.

Local saves and completed jobs wake the outbound poller in
[PR #150](https://github.com/krazyjakee/wobu/pull/150), with repeatable coverage recorded in
[the v0.1.11 evidence](13-acceptance-evidence.md#v0111--local-sync-and-concept-defaults).

### M4 — References

Image import via drag/paste, content-addressed hashing, thumbnails, the reference grid, per-image
role and weight, and Assets mode. Assets become real context, still with no generation.

### M5 — Enhance (first BYOK providers)

`wobu-llm`, keychain key storage, schema-constrained structured descriptions, streaming into the
editor, `stale` tracking, diff-and-accept on re-enhance. **Anthropic and Gemini ship together** so
the adapter boundary is real instead of one vendor's request shape wearing a trait.

### M6 — Influence Engine + first images

`wobu-influence` (resolution, fragments, text *and* per-role image budgets, attribution), the
Inspector panel, the ComfyUI adapter plus Gemini image, output presets for
character/prop/environment, capability negotiation, the job queue with live previews, and the
Concepts grid. **This is the first complete loop.**

Concepts default to one image for every node type; larger presets remain selectable
([PR #150](https://github.com/krazyjakee/wobu/pull/150)).

The per-project spend ceiling and cost estimate shipped here and were retired: they metered a local
model of published prices rather than the account that actually holds the money, so the number they
enforced could disagree with the provider's own balance in either direction. Paid models are marked
as paid; the provider's dashboard is the only thing that knows what is left.

Aspect choices come from the selected image backend. Unsupported or malformed saved values are
replaced before queueing, the UI previews the negotiated dimensions, and flexible backends use
Wobu's curated validated vocabulary ([#111](https://github.com/krazyjakee/wobu/issues/111)).

### M7 — Iteration and consistency

Forge mode, variant grids, seed locking, pin-to-reference promotion, and per-entity generation
history with replayable snapshots. The project-wide History mode shipped here and was retired by
[#144](https://github.com/krazyjakee/wobu/issues/144); receipts and replay stay on the Concepts
tab, where they belong to the entity they were generated for.

### M8 — Concept 3D

Turnaround preset → image-to-3D via Hunyuan3D — hosted BYOK and/or local weights under ComfyUI, per
[08](08-providers.md) — GLB storage in the project folder, in-app three.js viewer with turntable,
and export for a modeller.

The 3D tab is the whole of it: it surfaces every rendered turnaround view, re-rolls a bad one as a
single image on its own seed, gates a paid reconstruction behind an explicit confirmation — the
hosted backend bills per job and does not report the amount back, so consent is the only honest
gate — and queues the job beside the image ones. The finished GLB appears in the
viewer without a reload ([#110](https://github.com/krazyjakee/wobu/issues/110)).

### M9 — Later, if earned

| Feature | Status | Canonical issue |
| --- | --- | --- |
| Per-entity LoRA training | **Implemented** | [#69](https://github.com/krazyjakee/wobu/issues/69) |
| Relationship graph view | **Retired** | [#70](https://github.com/krazyjakee/wobu/issues/70), removed by [#144](https://github.com/krazyjakee/wobu/issues/144) |
| Cross-project style transfer | **Implemented** | [#71](https://github.com/krazyjakee/wobu/issues/71) |
| Multi-entity scene composition | **Implemented** | [#72](https://github.com/krazyjakee/wobu/issues/72) |
| Static world wiki export | **Implemented** | [#73](https://github.com/krazyjakee/wobu/issues/73) |

## Narrative extension in progress

[Narrative tracker #151](https://github.com/krazyjakee/wobu/issues/151) contains 16 concrete user
stories, the proposed workspace, and the implementation checklist. The
[narrative design](17-narrative-system.md) keeps the same contract in the repository. This extends
Wobu from concept assets to offline narrative compilation. N1 and N2 have working foundations;
their complete acceptance remains in progress. N5 is excluded from the current implementation scope.
Their N-prefix avoids collisions with the existing product and engineering milestone numbers.

| Milestone | Status | Exit result | Implementation issues |
| --- | --- | --- | --- |
| [N1 — Narrative world and scene authoring](https://github.com/krazyjakee/wobu/milestone/15) | **In progress** | Find scenes in the Scene library; author through coordinated Flow/Script views with separate layout metadata. | [#152](https://github.com/krazyjakee/wobu/issues/152)–[#157](https://github.com/krazyjakee/wobu/issues/157), [#184](https://github.com/krazyjakee/wobu/issues/184)–[#186](https://github.com/krazyjakee/wobu/issues/186), [#191](https://github.com/krazyjakee/wobu/issues/191) |
| [N2 — Deterministic compiler and playable preview](https://github.com/krazyjakee/wobu/milestone/16) | **In progress** | Compile/export and play offline, inspect arc Flow and played routes, and repeat saved scenario tests. | [#158](https://github.com/krazyjakee/wobu/issues/158)–[#162](https://github.com/krazyjakee/wobu/issues/162), [#187](https://github.com/krazyjakee/wobu/issues/187)–[#188](https://github.com/krazyjakee/wobu/issues/188) |
| [N3 — Generation and editorial review](https://github.com/krazyjakee/wobu/milestone/17) | **In progress** | Generate, edit, approve, and lock dialogue and supporting text without changing authored logic. | [#163](https://github.com/krazyjakee/wobu/issues/163)–[#167](https://github.com/krazyjakee/wobu/issues/167) |
| [N4 — Incremental builds and narrative analysis](https://github.com/krazyjakee/wobu/milestone/18) | **Planned** | Rebuild affected content safely, inspect bounded coverage and Flow diagnostics, and author the DSL. | [#168](https://github.com/krazyjakee/wobu/issues/168)–[#172](https://github.com/krazyjakee/wobu/issues/172), [#189](https://github.com/krazyjakee/wobu/issues/189) |
| [N5 — Portable engine integrations](https://github.com/krazyjakee/wobu/milestone/19) | **Excluded from current work** | Play matching native traces in Unity, Godot, and Unreal; export supported graphs to Yarn. | [#173](https://github.com/krazyjakee/wobu/issues/173)–[#177](https://github.com/krazyjakee/wobu/issues/177) |
| [N6 — Production pipeline and release readiness](https://github.com/krazyjakee/wobu/milestone/20) | **Planned** | Localise, voice, build reproducibly in CI, recover shared work, and record release acceptance. | [#178](https://github.com/krazyjakee/wobu/issues/178)–[#183](https://github.com/krazyjakee/wobu/issues/183) |

The current [authoring and Preview increment](19-narrative-authoring.md) advances world records
(#155), typed scene forms (#156), compiler/runtime foundations (#158/#159), playable Preview
(#161) and quest discovery (#191). The guide separates implemented behavior from pending
acceptance. Source repair, native package export, explicit save migration and evaluated Preview
traces, saved regression scenarios, attributed frozen context and portable record sync/recovery are
implemented. The native Flow spike records the selected toolkit and node budget. Cancellable provider
jobs produce separate prose proposals and immutable receipts. Editorial review and dependency
analysis remain planned. N5 (#173–#177) is not implemented, as requested.
