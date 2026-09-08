# 34 — Narrative domain model and the source/runtime type boundary

The `wobu-narrative` crate is the typed record of what a writer authored, before anything has been
compiled, generated or played. It is pure data: no IO, no async, no Tauri, no provider. It depends
on `wobu-core` for exactly one thing, the `EntityId` of a character who is already in the project,
because a scene's participants are the world's characters rather than a second copy of them.

This document states the model's contract and, in particular, where the versioned source types stop
and the runtime types begin. Storage is [narrative storage](24-narrative-storage.md); compilation
and execution are the [runtime contract](17-narrative-runtime-contract.md); version 2 migration is
[narrative source version 2](29-narrative-source-v2.md).

## Records

| Record | Holds | Identity |
| --- | --- | --- |
| Fact | The canonical assertion, its sources and the entities it concerns | World `EntityId` |
| KnowledgeClaim | One character's belief about one fact, its provenance and when it applies | World `EntityId` |
| Relationship | A directed `from`/`to` pair, a named kind, a typed value and a condition | World `EntityId` |
| WorldEvent | A summary, the facts it establishes, the entities present and a condition | World `EntityId` |
| Quest | Declared stages, an initial stage, conditional transitions and scene membership | World `EntityId` |
| FutureRestriction | A fact withheld from named characters until an explicit condition | World `EntityId` |
| Act / arc / tag | A named classification a scene may reference | World `EntityId` |
| Scene | Classification, participants, an entry condition, ordered beats and tombstones | `SceneId` |
| Beat | Intents, must-convey and must-not-reveal prose, dialogue, choices and outcomes | `BeatId` |
| DialogueSlot | A speaker, a generation policy and ordered variants | `DialogueSlotId` |
| Variant | A condition and one wording with its revision, provenance and lifecycle | `VariantId` |
| Choice | A player-facing label, a requirement, effects and a destination | `ChoiceId` |
| Outcome | An automatic condition, effects and a destination | `OutcomeId` |
| TextAsset | A supporting kind, host trigger, cast, selection policy and entries | `TextAssetId` |
| TextEntry | One alternative delivery of an asset: a condition and ordered lines | `TextEntryId` |
| Tombstone | What a deleted beat, slot, variant or entry was, and when it went | The deleted id |

Beliefs are independent of canon. A character may be certain of something false, and the same
character may hold true and false accounts of one fact under different conditions; neither changes
the fact and neither is a diagnostic, because deciding which account applies needs a playthrough's
state. What is diagnosed is a reference that can never resolve — a rumour attributed to a character
who does not exist is reported against `provenance.by`, not against the claim as a whole.

Relationships are directed and are never mirrored. Authoring "Kael distrusts Mira" does not
manufacture the reverse record, because the reverse is a separate authored statement.

## Typed state

Every state variable is a boolean, a declared enum, or an integer with an inclusive `min`/`max`.
There is no string, no float and no list: an open-ended domain cannot be enumerated, and bounded
variant planning and coverage analysis both stop being able to say anything about a scene that
mentions one. Each variable declares a required default — a missing initial value is the classic
source of a branch that behaves differently on a first playthrough than on a reload — and exactly
one owner. `Narrative` variables are written only by effects; `Host` variables are supplied by the
game, may be read by conditions, and an effect that assigns one is refused at authoring time.

Names are `[a-z][a-z0-9_]*` and may not be a YAML bare word (`on`, `off`, `null`, and the rest),
because such a name round-trips through the file as `true` or `null` and silently stops matching the
comparison written against it.

Conditions are boolean operators over comparisons of declared variables against literals or other
declared variables. Effects assign, add a fixed amount, or name a host command with arguments.
There is no `Call`, no `Script`, no `Raw(String)` and no arithmetic in operands. That closure is the
portable contract: a compiler that terminates in CI, an engine adapter with no interpreter in it,
and coverage analysis by finite enumeration all stop working the moment one variant holds arbitrary
code. Equality is defined on every type; ordering only on integers. Two enums compare only when they
declare the same members.

**Prose cannot author executable effects.** Intents, must-convey, must-not-reveal, summaries, roles
and destination labels are never parsed, matched or consulted by anything that decides where the
story goes, and dialogue strings are never interpreted as logic or commands. A supporting text asset
has no field a destination could be put in, so standalone prose exists without inventing a player
choice to hang it on.

## Three kinds of state

| Layer | What it is | Where it lives | Who may change it |
| --- | --- | --- | --- |
| Authoring canon | Facts, beliefs, relationships, events, quests, restrictions, scenes and text | Project files, read by `wobu-narrative` | A writer, through a guarded save |
| Scenario state | A validated starting assignment for a test or a Preview run | Saved scenarios, applied at `start_with_state` | A designer, per run |
| Runtime state | Cursor, variables, visits, selected variants, pending commands and seed | A `Snapshot` in the game's save | The runner, deterministically |

Canon does not move when a playthrough does, a scenario override never edits a declared default or
the compiled graph hash, and Preview never writes canonical project state. The layers are separated
by crate boundaries rather than by convention: nothing in `wobu-narrative` evaluates a condition,
resolves a branch, picks a variant or advances a cursor.

## Ordering and acquisition rules

- **Initial state.** A run initializes narrative variables from their declared defaults and requires
  every host variable explicitly. A quest starts at its declared `initial` stage, which must be one
  of its declared stages. Scenario overrides are validated and applied before any scene entry
  condition is evaluated. Unknown variables, wrong types and out-of-range values fail
  initialization.
- **Author order is the order.** Beats, dialogue slots, variants, choices, outcomes and text entries
  are ordered by their position in the list; there is no `order` field that could disagree with the
  position it is stored at. Reordering beats changes no destinations, and the first authored beat is
  a scene's entry point.
- **Events are not a chronology.** `WorldEvent` records what happened and under which condition. The
  model has no wall clock, infers no event ordering, and applies no "most recent wins" heuristic.
  Authors encode chapter and time windows in declared variables and conditions. Attending an event
  does not manufacture knowledge of it.
- **Knowledge is acquired only by an authored claim.** A claim names its character, its fact, the
  belief held (`true`, `false`, `unknown`), its provenance (witnessed, told by a named character,
  a rumour with a source, or inferred with a reason) and the condition under which it applies.
  Knowledge belongs to the speaker alone; another participant's knowledge is never inherited.
- **A future fact is not an established past.** A `FutureRestriction` withholds a fact from the
  listed characters — an empty list means everyone — until an explicit `until` condition. The
  condition is required rather than defaulted, because `never` is a restriction held indefinitely
  and an absent condition would be indistinguishable from an author who forgot. An active
  restriction's fact cannot also appear as usable knowledge, and an invalid release condition fails
  closed.

## Identity and revision

An **id** names a slot. It is minted once from the clock and a random source and never changes.
A **revision** names wording: it is a domain-separated BLAKE3 digest over the text and its
provenance, is derived rather than chosen, and changes on every edit. The types enforce the split —
each id has `new()` and no way to derive one from content, and `Revision` has only `Revision::of`.

| Operation | Ids | Revisions |
| --- | --- | --- |
| Rename a scene, beat, entry or classification | Unchanged | Unchanged |
| Reorder beats or entries | Unchanged | Unchanged |
| Rewrite a line | Unchanged | New; the approval falls back to draft |
| Duplicate a beat, scene, entry or asset | Fresh throughout | Carried over with the wording |
| Delete a beat or entry | Tombstoned, references left broken | Retained on the tombstone's label |

Ids derived from content would change when a writer fixed a typo, orphaning the translation, the
recorded audio and the lip-sync data keyed to the line. Revisions minted at random would make it
impossible to say whether the wording a reviewer approved is the wording about to ship.

A stored revision that no longer describes its stored words is reported as a diagnostic and is never
repaired in passing: resealing it is what would break the locale pack, so it is an explicit
decision. Deleting a beat leaves tombstones for it, its slots and its variants, and deliberately
does not rewire or clear destinations that named it — silently repairing them would delete the
author's statement about where that branch went.

## The versioned source boundary

Each document type carries its own `schema_version` and its own ceiling. They are independent so
that adding a field to one does not make every file of the other three look like it needs migrating,
and so that a project containing no supporting text at all stays readable by a build that has never
heard of it.

| Document | File | Reads | Writes |
| --- | --- | --- | --- |
| `SceneDocument` | `narrative/scenes/<slug>.yaml` | 1, 2 | 1 or 2; a new document is stamped 2 |
| `WorldDocument` | `narrative/world.yaml` | 1, 2 | 1 or 2; `for_save` stamps 2 |
| `StateDocument` | `narrative/state.yaml` | 1 | 1 |
| `TextAssetDocument` | `narrative/texts/<slug>.yaml` | 1 | 1 |

A document keeps the version it was read with. Only an explicit guarded save upgrades it, and a
version 1 document that has acquired a version 2 field is refused at both boundaries with the
remedy — declare `schema_version: 2` — rather than being upgraded silently.

These are all separate from `wobu_core::SCHEMA_VERSION`, which versions the project folder and the
existing world files. Narrative source is additive to a project that may contain none of it, and
bumping one number must not make every existing art project look like it needed a migration.

Four rules hold identically for all four documents:

- **Unknown fields are refused.** Every struct denies them. Serde's default of skipping what it does
  not understand would mean a mistyped key — or a key written by a newer Wobu — is dropped on the
  next save with nobody told. A rejected file can be fixed; a silently truncated one is data loss
  discovered months later as a branch nobody can find.
- **The version is read before the shape.** A permissive probe reads `schema_version` alone, so a
  file from a newer build is reported as a newer file rather than as thirty unknown fields. The
  message names the remedy, because the obvious response to a shape error is to delete the fields
  this build does not recognise, and that turns a newer project into an older one with data missing.
- **Unsupported versions are refused at both boundaries.** Reading and writing both reject a version
  outside `1..=ceiling`, including `0` and a missing version, which are their own errors rather than
  a guess. A reader that rejects what the writer emits is a project that saves once and never opens.
- **Migration is explicit.** Opening a project, rebuilding its index, reading Source, compiling or
  inspecting context does not migrate a file. Version 1 source rejects version 2 fields even when
  they are empty, and an unsupported document cannot be downgraded through a write boundary.

## The runtime boundary

Compilation is the one-way door. `wobu-narrative-compiler` lowers source into a runtime IR and
`wobu-narrative-runtime` executes it; neither reads this crate's authoring-only records, and nothing
in the runtime can produce a source document.

| Crosses into the graph | Stays in source |
| --- | --- |
| Declared typed state and command signatures | Variable descriptions |
| Scene entry conditions and explicit beat graphs | Intents, summaries, must-convey, must-not-reveal |
| Dialogue strings, revisions and stable string ids | Provenance, generation policy, review state, freshness |
| Branches, effects and destinations | Facts, beliefs, relationships, events, restrictions |
| Text assets with triggers, conditions and policies | Tombstones, editorial history, receipts |
| A source map from graph ids to scene/beat/slot ids | Canvas layout, which is a separate sidecar |

World records are authoring and generation-context inputs; they never become runtime graph
structure. Layout is presentation metadata keyed by stable ids and is excluded from semantic source,
compiler hashes and runtime exports, so moving a box cannot make dialogue stale.

The runtime carries its own versions — a graph version, a snapshot version and a package capability
manifest — and they do not move when a source version does. A `Snapshot` names the graph hash it was
taken against; ordinary `restore` rejects a different graph, and content migration is an explicit
opt-in hook rather than an unchecked deserialization path. The frozen generation request's
`source_schema_version` describes the source-language capability used to interpret its input, not a
uniform version for every file it read.

## Verification

`wobu-narrative`'s own fixtures cover the identity rules above (rename, reorder, duplicate, delete
and the tombstones deletion leaves), YAML round trips that preserve every id and every byte of
unicode and whitespace, malformed references and malformed types in scenes, supporting text and
world records, false and unknown belief with each provenance, conditioned contradictory accounts of
one fact, future restrictions, and the version matrix above at both the read and write boundary for
all four document types. Compiler, runtime, storage and context fixtures live with their own crates
and are described in the documents linked at the top.
