# Narrative source version 2

Scenes and World now read document versions 1 and 2. Explicit guarded saves write version 2.
State declarations remain version 1. Opening a project, rebuilding its index, reading Source,
compiling or inspecting context does not migrate files. Existing readers reject version 2 through
their unsupported-version check before attempting to interpret its fields.

This is a shared model checkpoint for #154, #186 and #194. Classification authoring, Scene library
facets and the complete Flow interactions follow in those issues; this checkpoint does not claim
that their UI or native acceptance is complete. Supporting text records are versioned separately;
see [supporting text](33-narrative-supporting-text.md) and the
[narrative domain model](34-narrative-domain-model.md). N5 is excluded.

## Explicit unfinished wiring

A route can now contain `to: {unresolved: {}}`. It retains its choice/outcome ID, label, condition and
effects while awaiting a destination. Its diagnostic addresses the route's `to` field with code
`unresolved_destination`. This differs from a beat with no routes (`no_destination`).

`to: {end: {label: Farewell}}` remains a deliberate ending. Deleting a beat retains destinations that
name the deleted ID and its tombstone diagnostic; deletion does not automatically disconnect or end
those routes. Scene duplication preserves unresolved routes and remaps internal beat references.

Both Development and Release compilation reject unresolved routes. Checked lowering never turns one
into a runtime End or emits a partial graph. Runtime Graph version 1 and native package capabilities
are unchanged; unresolved wiring is strictly an authoring state.

## Stable organization metadata

Scene has optional `act_id`, `arc_id` and `tag_ids` fields. They reference named records in World's
`acts`, `arcs` and `tags` arrays, each with `{id, name}`. Act and arc are independent classifications;
tags allow several memberships. Changing a display name retains the ID. Duplicating a scene retains
its classifications while allocating new narrative identities. Quest membership remains exclusively
in `Quest.scene_ids`.

Unknown references and repeated tag references produce field-addressed authoring diagnostics.
Duplicate World record IDs and empty names remain visible draft diagnostics. These records do not
become runtime graph structure. Review refuses to attest against invalid classification references;
repair the reference or remove it explicitly.

## Compatibility and immutable history

Version 1 source rejects the new fields, including explicitly empty fields, and rejects Unresolved.
Source authors introducing these fields must declare `schema_version: 2`. Unknown fields and future
versions continue to be rejected. Ordinary typed saves upgrade supported documents at their existing
stamp-guarded write boundary. An unsupported document cannot be downgraded through that boundary.

New optional fields default to empty and are omitted from serialization when empty. Consequently,
legacy embedded Scene values retain their serialized shape in editorial receipts and frozen context.
Existing World values keep the version they were read with. The semantic default for an absent World
remains version 1 until explicit creation/save, so opening an older project does not change its
reviewed context. Existing immutable receipts are never rewritten during migration.

The frozen generation request's `source_schema_version` describes the source-language capability
used to interpret the frozen input, rather than claiming a uniform version for every source file.
New plans use capability 2, including when they read version 1 files. Existing capability-1 requests
remain readable and keep their exact hashes; their original source stamps and context dependencies
still bind the actual input. Request/receipt, prompt, output and FrozenContext versions do not change.
A completed publication remains completed after reopening; an upgrade never authorizes another paid
request or modifies an existing frozen request in place.

The current conservative ReviewContext projection includes organization and destination edits.
Those edits, or an explicit World document upgrade, may make wording stale, including Locked wording.
The text, stable identity and lock remain intact. Scene envelope-only migration leaves the embedded
scene context unchanged. No freshness flag is cleared automatically and no field-precision claim is
made for #168.
