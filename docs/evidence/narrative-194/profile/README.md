# Read-path profile and budgets (#194)

These are measurements of real files through the public storage APIs, **not native UI acceptance**.
Product baseline: `730ab3a`. The example was built after the local crate-boundary bootstrap at
`4060079`; those scaffolds do not change the measured read path. Rust debug, pinned toolchain
`1.97` (resolved `rustc 1.97.1`). N5 is excluded.

[Budgets](budgets.json) were recorded before optimization. They cover actual native search/filter,
open/return, Review, frames, retained memory and bounded rendering. Unmeasured budgets remain open.
The first store profile isolates phases; three observations are enough to locate the repeated work,
and are **not** a p95 compliance claim.

## Reproduction and scope

Build `cargo build -p wobu-store --example narrative_read_profile` from `src-tauri`, then run its
binary with a **copy of the synthetic** 1,000-scene / 50,000-slot fixture. The example uses the normal
project-ID-local index and can update that disposable index during open/reconcile. It verifies the
source fingerprint is unchanged after all iterations. Never run this preparation against user data.
The selected fixture has 50 dialogue slots, no proposal/approval history, and one exact search result.

The run used `/tmp/wobu194-profile/narrative-scale.wobu`, copied from the original scale fixture.
The selected ID was `00000000000000000000000732`. Root Cargo/npm checks were paused for the
64.42-second run. Other desktop/user processes remained active; their process list and load averages
are in [environment](baseline-environment.json). CPU percentages there are process lifetime averages,
not an instantaneous CPU attribution. Index and disk cache temperature were not controlled, so the
22.4-second project-open sample is not labelled cold-index or cold-disk performance.

[Raw phase JSONL](baseline-store.jsonl), [sample summary](baseline-store-summary.json), and
[process resource usage](baseline-store-time.txt) retain every sample. Median times:

| Phase | Milliseconds |
| --- | ---: |
| Reconcile plan / decode index baseline | 416 |
| Reconcile observe / read and parse source | 5,795 |
| Reconcile revalidate / repeat source observation | 6,019 |
| Reconcile apply / compare index baseline | 1,017 |
| Selected scene load | 55 |
| Source fingerprint / scene catalog | 32 / 49 |
| Review capture | 236 |
| Proposal enumeration (empty proposal corpus) | 15 |
| Review view / current contexts (50 lines) | 70 |
| Approval evidence (no approvals) | < 0.01 |
| Final snapshot verification | 31 |
| Exact-wording Library query (one row, 3,677 JSON bytes) | 178 |

Reconciliation accounts for approximately 97% of the measured reconcile-plus-Review sequence.
`registry::observe` parses every source on both passes; plan/apply also decode the full indexed JSON.
This confirms a repeated reconciliation cost, rather than per-line context construction dominating
this fixture. It does not measure nonempty proposal scans or approval/history verification.
Peak RSS was 596,172 KiB for the standalone store process, not combined Tauri/WebKit memory.

The selected scene load was already 55 ms here. The earlier **native** uncached-open result near
3.9 seconds remains a scheduling/IPC/UI investigation; this profile does not attribute that delay to
the selected scene's YAML parsing. The earlier native Review samples near 20–21 seconds remain in
`../integrated-native-ui.json`; these separate measurements are not interchangeable.

## Approved implementation boundary

First optimize repeated parsing with a bounded, process-local cache of independently validated
source content. Every observation/revalidation must still enumerate canonical safe paths and
read/hash actual bytes. Keys include exact content and path/kind; errors/absence are not cached as
authority. Receipt/publication/reference checks must remain current. Account retained structured
allocations and entries, isolate caches by project session, and avoid cloning the full graph per pass.
Compare exact indexed row bytes for overlap detection without repeatedly decoding their full JSON.

Move expensive read work to workers with the project ticket captured before scheduling. Context and
evidence work outside the mutex uses immutable captured inputs and final ticket/observation checks.
Keep these changes separate from #170 materialization and #179 media/package writes. Add real-file
regressions for same-size edits, deleted/duplicate members, changed referenced receipts and index
mutation between plan/apply. Reprofile before expanding to shared batch captures or query work.

## Source-cache slice

The first optimization keeps independently parsed, valid Scene/Text/World/State entries in a
session-owned cache. Both passes still enumerate safe paths and read/hash every file. Records,
publications and receipt bindings always rerun reference validation. Immutable `Arc` values avoid
cloning the structured graph in each observation. Index overlap guards now hash the exact row bytes;
a same-hash metadata change still rejects an old plan. Local watcher reconciliation uses the same
content-checked path. No canonical source/schema format changed.

Before optimized measurements, the cache was capped at **2,048 entries and 384 MiB of conservatively
accounted structured allocations**. The initial proposed 256 MiB cap was raised before any optimized
run: a representative 50-slot scene accounts for 324,349 bytes, approximately 309 MiB across this
fixture. Accounting includes owned string/vector capacities, entry/Arc allocations and conservative
B-tree spare-node/link storage. Oversized entries remain in the current observation but are not
retained; eviction never removes a scene from results. This cache cap does not change the predefined
**1 GiB combined native/WebKit RSS budget**. Current observations and other application allocations
are additional memory; the standalone process measurement below does not prove combined compliance.

[Optimized raw samples](cached-store.jsonl), [summary](cached-store-summary.json),
[environment](cached-environment.json), and [resource usage](cached-store-time.txt) retain the complete
three-iteration run. No root Cargo/npm checks ran during its 11.87 seconds; unrelated desktop activity
remained. Median plan/observe/revalidate/apply fell to **21.9 / 44.2 / 43.1 / 22.1 ms** (approximately
131 ms total). Every iteration's final source-fingerprint assertion passed. Standalone peak RSS fell
from **596,172 to 195,072 KiB**. Project open was 8.80 seconds with uncontrolled index/disk cache
state. Review capture remained 263 ms and view contexts 72 ms; those were not optimized in this slice.
The first Library query was **485 ms**, followed by 199/203 ms; the outlier is retained, not discarded.
These three samples establish the phase improvement, not native p95 acceptance.

Validation: 253 store unit tests; four real-file cache/reconciliation regressions; eight existing
record/publication tests; ten Library integration tests; store all-target Clippy and workspace
formatting passed. New regressions cover same-size source edits with the exact original mtime,
deleted/duplicate scene membership, tampered referenced receipt bytes, stale index plans including
same-source-hash row changes, malformed-source non-retention and memory/entry limits. The original
proposal/approval-empty fixture still leaves nonempty history/batch performance to a later measured
slice. Captured-ticket native worker reads and final integrated measurements remain outstanding.

## Scheduled reads and coherent batch verification

Scene, Library, single-source Review/context and the project Review queue now schedule their work
on blocking workers with the exact open-project ticket captured before scheduling. Review context
computation uses immutable snapshots outside the project mutex, then checks the original ticket and
canonical observations before returning. Closing/reopening the same folder invalidates the ticket.
Capture/index work still uses the project mutex; this change does not claim every command has zero
lock contention.

Batch capture and final verification share one corpus fingerprint plus the union of exact
source/receipt and present/absent character observations. Mixed revisions or conflicting observations
fail. The queue retains its 32-container/10,000-line caps, cursor and collision checks, per-source
errors, and final proposal-list comparison. Its normal path captures the page together; malformed
capture falls back to per-source reads so one broken file does not remove other reviewable results.
Scene and supporting-text export evidence use the same batch boundary, including equality with the
captured source that will be compiled. Policy/approval/publication writes retain their guards.

Validation for this slice: 22 store Review tests, six locale tests, three supporting editorial tests;
16 application state tests, five queue tests and five export tests; store/application all-target
Clippy and workspace formatting passed. The new cases exercise late receipt changes without a
source-fingerprint change, absent-character membership, mixed capture revisions, source edits after
queue computation, and closed/reopened session rejection. Code-health remains at its unchanged
baseline. Native read, frame and combined RSS measurements, plus an actual approve/unlock/save
measurement, are still required before any interactive-budget claim.

## Integrated native follow-up

The [bounded current native measurements](../current-native/README.md) retain 30 read samples per
group, exact source-hash invariance, frame/RSS observations and one guarded write sequence. Search,
combined-filter and Review p95 still miss their predefined budgets. Writes take 23–31 seconds,
post-action summed RSS exceeds 1 GiB, and the short Script DOM-open observation includes a 525 ms
frame interval. These results do not close #194 or establish the complete #182 workflow.
