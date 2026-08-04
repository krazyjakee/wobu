# Wobu

Wobu is a local-first world building tool for producing concept art and concept 3D art. You author
your world once — art style, canon, species, cultures, places — and every image you generate
inherits that context automatically.

## The idea in one paragraph

Every image tool treats a prompt as a blank page. But world building is not a sequence of unrelated
prompts — it is a **tree**. A character belongs to a species, which belongs to a world, which is
rendered in a house art style. Today that context lives in your head and gets retyped, badly and
differently, into every prompt. That is exactly where visual consistency dies.

Wobu asks you to write it down once, at the level where it belongs. Style on the Art Style entity.
Anatomy on the species. Costume on the culture. Personality on the character. When you press
**Generate**, the prompt is compiled from that whole chain — and you can see exactly which layer
contributed which words.

## Start here

| Page | What it covers |
| --- | --- |
| [Your first project](getting-started.md) | From an empty folder to a world tree that produces consistent images. |
| [The workspace](workspace.md) | The rail, the navigator, the editor and the inspector — where everything lives. |
| [Entities and hierarchy](world-model.md) | The ten entity kinds, how they nest, and how influence flows between them. |
| [The influence stack](influence.md) | How your notes become a prompt, and how to steer it without rewriting the world. |

The first time you open Wobu it walks you through this itself: two legal documents to accept, then a
short introduction to the four modes, opening a project, and your keys. You can skip the
introduction at any point and run it again later from the launcher or from Settings.

## What makes Wobu different

### The hierarchy is the product

Not the image grid. The value compounds as the tree grows — the hundredth character is cheaper and
more consistent than the first, because by then the world already knows what your species look like,
how your cultures dress, and what light your coast gets.

### Notes in, canon out

You write rough, messy, half-sentence notes. **Enhance** turns them into a structured canonical
description. Your messy notes are never overwritten — they remain the source. Nothing Enhance
produces is written to disk until you have read it and accepted it, section by section if you like.

### Nothing is hidden

The compiled prompt is always visible and always attributed. There is no invisible prompt-magic, no
secret quality suffix, no reordering you cannot see. If an image came out wrong, you can point at
the layer that caused it.

### Images are context, not decoration

A reference image carries a **role** — silhouette, palette, material, mood, pose, costume or full
reference — so the compiler knows whether to route it to a style adapter, a structure adapter, or
just your own mood board.

### Local first, and yours

A project is a folder on disk. Notes are Markdown. Images are files. It works offline against a
local GPU, it opens in Obsidian, it versions in git, and it survives Wobu being uninstalled. This
guide is inside the application, so it works with no network at all.

> **Bring your own key** Wobu operates no inference of its own and no proxy. Nothing is
> pre-configured and there is no account: you supply credentials you obtained yourself, and Wobu
> talks to those providers directly on your behalf. Keys live in your operating system's keychain
> and never enter the project folder. Until you add one, Enhance and Generate are switched off and
> say why — everything else in the app works. See [Providers and keys](providers.md).

## What Wobu is not

| | |
| --- | --- |
| Not a finishing pipeline | Output is concept art. The 3D tab reconstructs blockout meshes from a turnaround and exports them. Neither is a production asset. |
| Not a wiki or a novel-writing app | Notes exist to drive images. If a fact cannot shape a render, it does not need to live here. (There is a static wiki *export*, for showing the world to other people.) |
| Not a node-graph tool | ComfyUI already exists, and Wobu can drive it. Wobu's surface is a document editor, not a canvas of wires. |
| Not real-time multiplayer | Several people can share one project folder, and copies can sync peer-to-peer. But there is no co-editing cursor and no server. See [Sharing a project](collaboration.md). |
| Not a telemetry client | Wobu reports nothing, anywhere, ever. The only outbound requests are the ones you cause by pressing Enhance or Generate. |
