# 01 — Vision & Principles

## The problem

Every image-generation tool treats a prompt as a blank page. But world building is not a
sequence of unrelated prompts — it is a **tree**. A character belongs to a species, which
belongs to a world, which is rendered in a house art style. A prop belongs to a culture. A
room belongs to a city belongs to a region.

Today that context lives in the artist's head and gets retyped — badly, inconsistently, and
differently every time — into each prompt. That is exactly where visual consistency dies.
Two characters of the same species end up looking unrelated. The lighting drifts. The
palette drifts. By image forty, the world has no identity.

## The bet

**Author the hierarchy once. Every generation inherits it.**

Wobu is an authoring tool first and a generation tool second. You spend your time writing
notes and collecting reference images at the level where they belong — style at the project
level, anatomy at the species level, costume at the culture level, personality at the
character level. When you finally hit *Generate* on a character, the prompt is compiled from
that whole chain, and you can see exactly which layer contributed which words.

## Principles

1. **The hierarchy is the product.** Not the image grid. The value compounds as the tree
   grows — the hundredth character is cheaper and more consistent than the first.

2. **Notes in, canon out.** You write rough, messy, half-sentence notes. *Enhance* turns them
   into a structured canonical description via an LLM — and that description is itself
   editable and version-tracked. The messy notes are never overwritten; they are the source.

3. **Enhanced descriptions are structured, not prose.** `Silhouette`, `Anatomy`, `Materials`,
   `Palette`, `Signature details`, `Never`. Structure is what lets the prompt compiler pick
   the right fields for a full-body shot versus a material study.

4. **Nothing is hidden.** The compiled prompt is always visible, and every fragment is
   attributed to the layer it came from. Any layer can be muted or weighted per generation.
   No invisible prompt-magic.

5. **Images are first-class context**, not decoration. A reference image carries a *role* —
   silhouette, palette, material, mood, pose — so the compiler knows whether to route it to a
   style adapter, a structure adapter, or just the mood board.

6. **Local first.** A project is a folder on disk. Notes are Markdown. Images are files. It
   works offline against a local GPU, and it survives Wobu being uninstalled.

7. **Iterate cheaply, promote deliberately.** Generations are disposable by default. Pinning
   one promotes it to a reference image on its entity — which then influences everything
   downstream. That is the flywheel.

## What Wobu is *not*

- **Not a finishing pipeline.** Output is concept art and concept 3D — blockout meshes and
  turnarounds for a modeller to work from, not production assets.
- **Not a wiki or a novel-writing app.** Notes exist to drive images. If a fact can't shape a
  render, it doesn't need to live here.
- **Not a node-graph tool.** ComfyUI already exists and Wobu can drive it. Wobu's surface is
  a document editor, not a canvas of wires.
- **Not multiplayer** (v1). Single artist, single machine, git-friendly on disk.
