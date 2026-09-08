# Narrative dependencies and Why affected

Changing a character's voice automatically marks the lines that read it out of date.
Narrative → Context → Why affected explains each change as **source field → context/query →
variant**. The old words, IDs, revisions, locks, approval decisions and provider receipts remain
available. This implements US-06 and N4 issue #168; build scheduling and Flow affected-scope
highlighting belong to #169.

## What a line depends on

`wobu-narrative-deps` captures two kinds of input from typed canonical narrative data:

- Field addresses and content hashes, including an explicit absent value for a failed lookup.
  Examples include character voice, beat intent, variant condition and source fact.
- Query parameters and pre-condition candidate membership for restrictions, speaker knowledge,
  directed relationships and relevant events. Adding a record can therefore affect a request
  that could not have named its previously nonexistent ID. Removing a member is also explained.

Capture considers every scenario's candidates, so it conservatively includes conditional
records even when one preview would not use them. Only declarations of state variables read
by a condition are dependencies; changing a preview slider does not change source freshness.
The context-resolver vocabulary contract tests verify query names, IDs and hashes agree.

BLAKE3 fingerprints include variant identity, source schemas, compiler/context/prompt/output
and dependency versions, and the generated wording's frozen provider/model/settings. Sorted
maps normalize query membership and settings object order. Handwritten text has no producer.
Scene sibling wording, review flags, asset-entry display labels and unused classification
fields are excluded. Ambient exchanges include ordered neighboring lines because their shared
generation context reads them; unrelated exchanges remain independent. Text repeat policy is
also an input. Scene names and beat titles remain inputs because the context resolver reads them.

`narrative_fingerprint` brackets source reads; `narrative_build_fingerprint` additionally
includes toolchain versions. Neither includes `narrative/layout/`. Typed dependency capture has
no filesystem input and cannot read layout. Layout-only saves therefore produce no dependency
edges, no new receipts and no freshness changes.

## Canonical history and a rebuildable index

Immutable `narrative_dependency_baseline` receipts in `narrative/receipts/` record the dependency
set and wording revision for each authored or explicitly reviewed variant. Authoring batches
create bounded receipt chunks below the existing 2 MiB sync limit, including envelope bytes;
they do not create a file for every line. These use
the ordinary immutable receipt bindings and project sync registry.

Every entry names its previous receipt. Causal ancestry chooses the current baseline, including
when a collaborator's clock runs backwards. Missing parents, cycles and concurrent heads are
reported instead of using timestamps to silently acknowledge one writer's context. All old
receipts remain canonical evidence.

SQLite `narrative_dependency` and `narrative_dependency_edge` are disposable projections of
these receipts. Cache rebuild reads the historical receipts, **not today's source inputs**.
Deleting the actual database after changing a voice produces the same original baseline,
affected variants and Why affected explanations when the project reopens. Ordinary reads only
replace the projection when its content changed.

Marking freshness never advances the baseline. Repeated marking is idempotent and preserves
explanations until new wording or an explicit review/attestation records the current context.
An attestation acknowledges only its selected variant. Imported or remotely observed wording
without historical evidence remains untracked until its canonical receipt arrives or a local
author explicitly edits or attests it. A watcher never invents a baseline during partial sync.

## Automatic propagation and publication guards

Successful scene, supporting-text, character and state/world saves refresh dependencies, as do
full scans and both watcher reconciliation routes. Deletions also propagate. Invalid files
remain visible through ordinary index diagnostics; automatic maintenance pauses rather than
publishing a baseline from a partial project. Explicit dependency reads still report the error.
Read-only projects are not modified.

Freshness writes have their own guarded path, change no words or editorial history, and can
mark locked text. The snapshot brackets source fingerprints and character file stamps, then
checks them again before baseline publication. Freshness publication compares the actual
container with the captured revision and uses the normal guarded writer. A stale snapshot
cannot publish a receipt or mark replacement wording on the strength of an old read.

Approvals and context attestations use version 2 review fingerprints over the same target
inputs plus only referenced scenario values. Rewriting a sibling line or adding an unrelated
fact leaves an approval valid. Changing a relevant voice or query membership withdraws release
readiness while retaining the decision. Version 1 contexts keep their original verification
rules; historical version 2 decisions verify using their recorded toolchain versions, while
current readiness uses the current toolchain.

Downstream production records are joined by their `variant_id` and reported as affected. No
production content is rewritten. This is the existing generic production-record contract;
no unimplemented recording/translation engine is claimed.

## Verification and limits

Pure dependency tests assert exact affected sets for voice, fact, relationship, event,
knowledge, condition, label, sibling-wording and prompt-version changes. Store tests cover
automatic local/watcher marking, locked approvals, precise attestation, actual SQLite loss
after a source edit, causal successors with backwards clocks, competing heads, stale snapshot
publication refusal, bounded batch enrollment and source-before-receipt peer delivery. Layout tests compare source, build,
context, receipt and dependency evidence before/after cosmetic edits.

The current capture remains proportional to lines × world records and has no incremental
scheduler. A focused 1,000-line batch took 5.35 seconds to author/save in a development build (two receipt
chunks), and verifies peer import plus reconstruction across both chunks; it is not a claim of native
UI responsiveness at 50,000 slots. Provider identities are read from frozen receipts; tests
make no live provider call. Native UI evidence is collected separately during integrated
workflow validation.
