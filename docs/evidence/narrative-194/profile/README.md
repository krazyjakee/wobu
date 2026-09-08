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
