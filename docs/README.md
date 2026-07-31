# Wobu — Design Documentation

Wobu is a local-first, AI-assisted world building tool for producing **concept art** and
**concept 3D art**. Its organising idea is a *hierarchy of influence*: you author the world
once — art style, lore, species, cultures, places — and every image you generate inherits
that context automatically.

| Doc | What's in it |
| --- | --- |
| [01 — Vision & Principles](01-vision.md) | The problem, the product bet, what Wobu is *not* |
| [02 — Data Model](02-data-model.md) | Nodes, links, assets, generations, on-disk format |
| [03 — UI Layout](03-ui-layout.md) | Screens, panes, components, design tokens, shortcuts |
| [04 — Influence Engine](04-influence-engine.md) | How context resolves into a prompt; the Enhance pipeline |
| [05 — Technical Architecture](05-architecture.md) | Tauri/Rust structure, adapters, jobs, events |
| [07 — Projects on File Shares](07-file-shares.md) | Presence, conflicts, network-mount behaviour, performance |
| [08 — Providers & BYOK](08-providers.md) | Key storage, Gemini, Hunyuan3D, capability negotiation, cost |
| [09 — Roadmap](09-roadmap.md) | Milestones from prototype to 3D |

Two constraints shape most of the above and are worth knowing up front:

- **A project is a self-contained directory** meant to live on a file share, so several people
  can open it. Nothing canonical is stored outside it.
- **All inference is bring-your-own-key.** Wobu operates no proxy and no inference of its own;
  keys live in the OS keychain, never in the shared project folder.

A clickable HTML/CSS prototype of the main workspace lives in [`/prototype`](../prototype).
Run it with `npx serve prototype` and open the printed URL.
