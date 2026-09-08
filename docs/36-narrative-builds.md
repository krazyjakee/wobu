# Affected narrative builds

Build → Affected content plans saved scene dialogue and supporting text together. Choose Missing
wording, Affected content, or All in selected scope, then select the rows to run. Planning uses the
configured text provider/model metadata without accessing its credential or making provider calls.
The table shows source reasons, Generated replacements, Edited proposals, Locked exclusions,
validation blockers, and reusable successful results. It displays 50 rows per page; selection and
counts cover the complete plan. Scenario tests remain available from Build.

Generation starts only through **Generate / resume selected**. Validation, scenario replay,
compilation and export continue to consume accepted source; they never fill missing text or call a
provider. Unsupported configuration, malformed source and unavailable context are reported before
spending. The current planning limit is 10,000 items; select a smaller scope above that limit. Queue
concurrency controls execution independently of this limit. The legacy single-scene generation
surface retains its 32-request bound.

## Durable completion and reuse

A build is an immutable receipt manifest with bounded item chunks. Every eligible item references
its own immutable frozen generation request, candidate identity and scenario state. The manifest is
published after the item chunks and requests, so an interrupted capture cannot expose an incomplete
build as resumable. Each file remains subject to the existing 2 MiB portable narrative-file limit.
A dispatch receipt records the selected requests before submission. Live job IDs stay in memory;
provider attempts, successful outputs and exact receipt/proposal publication pairs remain canonical
project files.

Reopen a saved build to inspect completion and explicitly select unfinished or failed items.
Completed requests never call the provider again. A successful attempt whose proposal publication
was interrupted is recovered through the existing generation publication path. Failed, cancelled or
interrupted calls may have incurred charges; another attempt is an explicit writer decision.
Opening a project, inspecting a build or restarting the application never queues paid work.

Matching successful output can be reused by another plan. A versioned reuse key covers authorized
context, target, text/policy guards, source capability, prompt/output versions, provider, model and
settings. Random request/batch IDs and unrelated container bytes do not identify reusable prose.
For an empty slot, reuse adopts the original request's reserved candidate identity. The original
request and validated output are retained; reuse does not invent another paid attempt or rewrite
its provenance. Missing or invalid output cannot count as a cache hit.

The reuse lookup is disposable in-memory data rebuilt from canonical evidence. No accepted wording,
review proposal, request, attempt, editorial receipt or production reference is a disposable build
cache. Rebuilding or replacing SQLite leaves these files intact. Accepted wording's producer is
resolved by its exact recorded request fingerprint, so planning with another model cannot relabel
already accepted prose.

## Guarded execution and publication

Planning resolves all targets from one coherent dependency snapshot. Dispatch rechecks its selected
items from shared captures before and after credential acquisition. The existing generation job
then rechecks current source, text revision, context and policy immediately before the provider
call. A source change or new lock prevents that call. Changes during a call retain the validated
result as evidence and a review proposal, while the editorial write boundary prevents replacement.

New version-2 generation requests guard the selected target instead of the entire container file.
Their frozen context keeps its complete integrity hash; semantic eligibility excludes the legacy
aggregate scene dependency while retaining the resolver's individual reads, fragments and queries.
An independent sibling's acceptance therefore does not invalidate the next planned line. Actual
context dependencies still do: changing an earlier ambient line can stop a later line whose prompt
read it. Replan that remaining work against the newly accepted context. Container locks and file
stamps remain the atomic editorial publication guards. Version-1 requests retain their original
whole-container/context checks and their existing serialized evidence.

An Edited asset, slot or variant retains a proposal. Automatic replacement requires Generated policy
at every applicable level. Any Locked level excludes generation. Explicit acceptance still goes
through the shared review transaction and preserves stable source identities.

## Verification and limits

Deterministic real-file tests cover the 483-item plan and reopening, portable receipt sizes,
Generated siblings, Edited proposals, new locks and source changes, successful-output recovery,
partial failure/cancellation with a fresh SQLite index, model/context cache misses, voice-change
scope, supporting/ambient dependencies, and historical request compatibility. Existing provider
job tests use scripted providers to verify call counts, cancellation, output validation and late
publication. Frontend tests cover explicit subset submission, retained completion, failed-item
retry and pagination across 483 rows. These checks are mocked-provider evidence, not live-provider
or game-engine validation.

The per-item state and candidate identity are inputs to this executor. Future bounded variant
planning (#170) can supply them; this change makes no reachability claim and does not enumerate
state partitions. Whole-project source capture and reconciliation still have costs on very large
projects; performance work is tracked separately in #182.

For a native walkthrough, create an existing temporary directory and run:

```sh
cd src-tauri
cargo run -p wobu-store --example affected_build_fixture -- /tmp/build-demo
```

Open the printed `.wobu` folder, then Build → Affected content → Plan work. Three voice-dependent
lines split across Generated, Edited and Locked; a fourth unrelated line stays outside the plan.
The example first saves their original context, then changes the keeper's voice. Pass `483` as the
last argument for the larger planning/pagination fixture. Reopen the saved plan to inspect durable
intent without starting generation. With no configured key, explicit generation reports the
missing credential and preserves that plan; no provider is invoked by the fixture itself.
