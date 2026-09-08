# Dependency tracking: what each line was written against, and what moved

Changing Kael's voice marks the lines that read it out of date and leaves every other scene
alone. That is US-06, and it is this page. It is N4 issue #168, and the limits at the end say
which parts of it are here and which are still #169.

## The one thing that is easy to get wrong

A dependency is not only a list of the records a line named. It is also the list of records a
line was *resolved against* — the candidate sets, before any condition was evaluated.

Consider a generation request that asked what Kael feels about Mira. It resolved
`directed_relationships` from Kael to the scene's participants, found nothing, and produced a
line. A month later somebody authors "Kael resents Mira". That record has an id no earlier
request could possibly have referenced, so a reverse index keyed on ids has nothing to look up
and reports nothing. The line is stale and the badge never appears.

So a dependency set has two halves.

- **Fields** — an address and the content hash found there:
  `character/<id>/narrative_voice`, `scene/<id>/beat/<id>/must_convey`,
  `world/facts/<id>`, `state/<name>`. A lookup that found nothing is recorded as an
  address with no hash, which is what makes *deleting* a fact a change rather than an
  absence of one.
- **Queries** — the whole pre-condition candidate set of each named query, member id
  beside member hash. Adding a record is a member appearing; removing one is a member
  vanishing; rewriting one is a member's hash moving. The names, the parameters and the
  filters mirror `wobu-narrative-context`'s exactly, because the frozen context on a
  generation receipt and the dependency set in the index have to be talking about the
  same world.

The queries are `restrictions`, `speaker_knowledge`, `directed_relationships` and
`relevant_events`. A test walks a real `resolve` and asserts the two crates produce the same
query names, the same member ids and the same hashes for the same records; a divergence there
would be an invalidation that silently stops reaching a receipt.

## Why the capture ignores the scenario

The context resolver answers what one request read, in one chosen scenario, at one instant —
the right shape for a receipt. An index cannot privilege one preview's variable values without
being wrong for every other, so the capture never evaluates a condition. Where the resolver
narrows `relevant_events` to the facts a scenario makes available, the capture takes every fact
the speaker's claims name. That is a superset, deliberately: the index can mark a line affected
that one particular preview would not have cared about, and it can never miss one.

It records the *declaration* of a variable a condition reads, never its value. Narrowing an
integer's range changes which branches can ever be taken; moving a preview slider does not.

## What is deliberately not a dependency

| Not tracked | Why |
| --- | --- |
| Another line's wording | Rewriting one line must not mark its neighbours stale. The review context that predates this hashes the whole scene document, so a typo fix withdraws every approval in the file; a dependency set contains no sibling's body, revision or provenance. |
| Editorial flags | Policy, review state and freshness are *outputs*. A fingerprint containing them would move every time it was acted on. |
| Display labels | A text entry's `label` derives nothing by its own definition. A beat's `title` is not in this category — the resolver puts it in the `objective` fragment, so a model saw it. |
| Act, arc and tag | Nothing in the resolver reads them. Filing a scene under a different act changes no word of it. |
| Flow layout | There is no field a coordinate could arrive through, and the capture reads no files at all. See below. |

## Fingerprints

Every set hashes to a BLAKE3 digest over its canonical JSON, domain-separated and
length-prefixed for the reason `Revision::of` prefixes: without it the boundary between two
parts is guessable and two different sets can be made to collide. It covers the target's own
variant identity, so wording moved to a new variant is a new slot with no inherited freshness.

It also covers the whole toolchain: the state, scene, world and text schema versions, the
context resolver, the compiled graph, the prompt, the output schema, the review record and the
dependency format itself. Each is a real input — a new prompt version asks the model for
something different, a new resolver assembles different fragments from the same source — and
leaving one out produces content that looks current, was made by a toolchain that no longer
exists, and would not be produced again. Provider, model and settings are recorded for wording
a model wrote, with settings normalised into a sorted map so two spellings of the same object
hash alike. Hand-written wording has no producer and does not acquire one when the project
switches models.

There are two project-wide fingerprints and they answer different questions.
`narrative_fingerprint` is bytes of source only — the right thing to bracket a read with,
because a coherence check must not fail merely because the app was upgraded mid-read.
`narrative_build_fingerprint` folds the toolchain in, because the same bytes compiled by a
different compiler are not the same build.

## Why affected

Every difference keeps the address it was found at, and renders as three legs: **source field
→ context or variant → line**.

```
character/01J…/narrative_voice  →  context                        →  scene/01K…/…/variant/01M…
01N…                            →  query directed_relationships   →  scene/01K…/…/variant/01M…
prompt                          →  toolchain                      →  scene/01K…/…/variant/01M…
```

The comparison is structural rather than a hash equality, and the two agree in both
directions: a reason with no fingerprint movement would be a change nothing keys on, and a
moved fingerprint with no reason would be a badge nobody can explain, which is what teaches
people to ignore the badge. **Narrative → Context → Why affected** shows these rows for the
selected line, in the backend's own words.

## Propagating an invalidation

Marking a line out of date sets one flag. The body, the revision, the provenance, the
generation policy and the review state are all left exactly as they were.

- A **locked** line can still be marked. `ContentLifecycle` is three fields precisely so that
  locking cannot make stale wording look current: a lock protects words, and this writes no
  words. The ordinary save path refuses any change to a locked variant's text and freshness
  lives inside that text, so this goes through its own writer — which still takes the scene
  lock and still refuses a losing write rather than merging it.
- An **approval** is retained and still verifies. Freshness is derived, so it is normalised
  away by the editorial-history check: a scene whose world moved has not been edited outside
  its recorded history, and reporting it as such would discard every binding in the file.
  The approval stays recorded and the line stops being release-ready, which is the pair a
  reviewer needs to see.
- **Production artifacts** (#178–#183) are reported and never rewritten. The recording of a
  line that has gone stale is still the recording that was made.
- An **untracked** line — new, or one the local index has never seen — is reported as
  untracked and nothing is written for it. Nothing is known about what it was written
  against, so nothing can be claimed to have gone stale.

## The index is a cache, and only half a projection

`narrative_dependency` and `narrative_dependency_edge` live in the local SQLite index, hold no
canonical data, and are always safe to delete. They are not in `CLEAR_DERIVED_SQL`, though,
and that is the interesting part: unlike `nodes` or `narrative_files` they do not describe what
the folder currently says, they describe what each line was written *against*, which is a claim
about the past that no folder records. They are the same category as `sync_state` — safe to
delete, not derivable — so a routine rescan must not quietly discard them.

Losing them is an explicit act, recovering from them is a capture from canonical data, and the
round trip is a test rather than a claim: the recorded index is dropped and the rebuilt one has
to be identical, edges included. After a real loss every line reports as untracked, which is a
full rebuild rather than a project that has silently gone stale.

The edge table is an accelerator and never a second opinion. Reading the index back re-derives
the edges from the sets, so a stored edge row can never be the thing a comparison depends on,
and a test asserts the two agree. The candidate lookup it serves is deliberately a *superset*
of the affected set: a filter that could exclude an affected line would produce a build that
silently skipped work.

## Coherent reads

The whole capture is bracketed. The source fingerprint and every character node's stamp are
taken before and compared after, and a snapshot that moved under the reader is refused rather
than published — a capture that read half a project before a colleague's save and half after
would describe a project that never existed and then confidently mark the wrong lines. This is
the same guard the context capture and the review snapshot use.

## Flow layout invalidates nothing

The capture takes scenes, supporting text assets, the world document, the declared state and
character voices. None of them has a field a coordinate could be written into, and the crate
reads no files, so `narrative/layout/` is unreachable rather than filtered.

The existing #185 evidence now covers this too: the cross-boundary contract test drags a node
on a real project and asserts that the source bytes, the receipts, the review view, the frozen
context, the compiled graph, the package bytes, **both fingerprints, the dependency index and
the affected set** are all identical afterwards.

## Verification and limits

Rust tests change a voice, add a relationship nobody ever referenced, remove one, add and
remove a fact a query reaches, add an event that becomes relevant by matching rather than by
being named, add a knowledge claim that widens the reachable fact set, rename display labels,
rewrite a neighbouring line, insert a shadowing variant, narrow a variable declaration and
upgrade the prompt version — each asserting an **exact** affected set, because a tracker that
marks everything affected passes every "did it notice" test ever written. Store tests drive the
same claims over a real project folder, including the locked-and-approved case, the cache-loss
round trip, the production-artifact report and the layout invariant. React tests mock IPC and
establish the rendered rows, not native Tauri rendering.

Not done in this checkpoint, and not implied by it:

- **Build planning.** Nothing here decides what to rebuild, in what order, at what cost, or
  queues a provider call. That is #169, and the Why affected pane says so.
- **Canvas highlighting.** Flow does not tint affected beats yet; the pane is in the Context
  inspector only.
- **Automatic marking.** Freshness is propagated when a caller asks for it. Nothing runs it on
  a save or a watcher event yet.
- **Live validation.** No provider was called. Producer identity is read from the frozen
  requests already on disk.
- **Screenshots.** No UI evidence is recorded for this checkpoint.
