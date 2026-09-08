# Wobu — Design Documentation

Wobu is a local-first, AI-assisted world building tool for producing **concept art** and
**concept 3D art**. Its organising idea is a *hierarchy of influence*: you author the world
once — art style, lore, species, cultures, places — and every image you generate inherits
that context automatically.

The [narrative system](17-narrative-system.md) now has scene/world authoring, typed dialogue
structure, source repair, portable sync/recovery, saved scenarios, native packages and isolated
deterministic Preview. Attributed context and cancellable prose-generation jobs are implemented.
Guarded editorial review is implemented; incremental and production builds remain planned.
The [authoring guide](19-narrative-authoring.md) records
the implemented behavior and remaining acceptance. N5 integrations are excluded from current work.

| Doc | What's in it |
| --- | --- |
| [01 — Vision & Principles](01-vision.md) | The problem, the product bet, what Wobu is *not* |
| [02 — Data Model](02-data-model.md) | Nodes, links, assets, generations, on-disk format |
| [03 — UI Layout](03-ui-layout.md) | Screens, panes, components, design tokens, shortcuts |
| [04 — Influence Engine](04-influence-engine.md) | How context resolves into a prompt; the Enhance pipeline |
| [05 — Technical Architecture](05-architecture.md) | Tauri/Rust structure, adapters, jobs, events |
| [07 — Projects on File Shares](07-file-shares.md) | Presence, conflicts, network-mount behaviour, performance |
| [08 — Providers & BYOK](08-providers.md) | Key storage, Gemini, Hunyuan3D, capability negotiation, cost |
| [09 — Roadmap](09-roadmap.md) | Canonical milestone status from foundations to 3D |
| [12 — Packaging & Releases](12-releasing.md) | Tagged bundles, signing and updater policy, version stamping |
| [13 — Acceptance Evidence](13-acceptance-evidence.md) | Repeatable M1/M6 contracts and the live smoke checks they cannot replace |
| [14 — Code-health Checks](14-code-health.md) | Dead-code, direct-dependency, public-surface, and duplication regression gates |
| [15 — Exit Policy](15-exit-policy.md) | Every way the app can be stopped, what is in flight on each, and what is flushed, cancelled or accepted as lost |
| [16 — Agent Access (MCP)](16-mcp.md) | The opt-in MCP server and client, what each exposes, and what guards the port |
| [17 — Narrative System](17-narrative-system.md) | User stories, implementation status, compiler/runtime boundaries, and remaining backlog |
| [17 — Runtime Contract](17-narrative-runtime-contract.md) | Internal compiler graph, runtime semantics, commands and snapshots |
| [18 — Narrative Source Editor](18-narrative-source-editor.md) | YAML policy, draft safety, diagnostics and round trips |
| [19 — Narrative Authoring](19-narrative-authoring.md) | Scene/quest discovery, World, typed Script, Preview, draft safety and acceptance gaps |
| [20 — Native Narrative Packages](20-native-narrative-packages.md) | Export profiles, native JSON layout, identity, validation limits and safe publication |
| [21 — Native Narrative Preview](21-native-narrative-preview.md) | Real Tauri walkthrough, command results, restore, bounds and project isolation evidence |
| [22 — Narrative Scenarios](22-narrative-scenarios.md) | Saved Preview runs, partial assertions, offline replay and first-divergence reports |
| [23 — Narrative Generation Context](23-narrative-context.md) | Speaker knowledge, attributed constraints, bounded frozen inputs and freshness |
| [24 — Narrative Storage](24-narrative-storage.md) | Canonical records, immutable receipts, publication and rebuildable local indexes |
| [25 — Narrative Generation](25-narrative-generation.md) | Frozen plans, provider jobs, protected proposals, cancellation, receipts and recovery |
| [26 — Narrative Review Evidence](26-narrative-review.md) | Canonical editorial history, guarded transitions and verified release approval |
| [27 — Shared Narrative Arrangements](27-narrative-layout.md) | Layout schema, groups/notes, peer merging, native canvas evidence and bounds |
| [28 — Narrative Review Queue](28-narrative-review-queue.md) | Revision comparison, protected drafts, paged review and grouped bulk decisions |
| [29 — Narrative Source Version 2](29-narrative-source-v2.md) | Explicit unresolved routes, stable classifications and backward-readable source migration |
| [30 — Shared Scene Editing](30-narrative-editing-session.md) | Shared drafts, explicit saves, local undo and retained conflicts |
| [31 — Scene Library](31-narrative-library.md) | Indexed discovery, classifications, saved views and scale evidence |
| [32 — Narrative Flow](32-narrative-flow.md) | Shared structural editing, explicit destinations, keyboard navigation and bounded arrangements |

Two constraints shape most of the above and are worth knowing up front:

- **A project is a self-contained directory** meant to live on a file share, so several people
  can open it. Nothing canonical is stored outside it.
- **All inference is bring-your-own-key.** Wobu operates no proxy and no inference of its own;
  keys prefer the OS keychain and use an owner-only app-data fallback when needed, never the shared
  project folder.
