# Bounded beat variants

A beat's **Variants…** matrix is an offline authoring tool. It saves an explicit analysis policy
and an immutable report in canonical project files. It makes no provider call. Local indexes can
be rebuilt without losing either record. **Materialize and open Build** first records an undoable
structural edit, then freezes each conditional variant with its complete witness state through the
existing Build queue. Generation still requires an explicit Build action.

The matrix displays potential, included, excluded and unknown configurations, concrete variable
buckets, witnesses, authored priority and configured limits. Counts are decimal strings so large
cardinalities do not round in the desktop bridge. A saturated cardinality is marked as a lower
bound. Tables paginate without limiting whole-matrix selection.

## The claim and its assumptions

This version proves configurations under an **explicit state-transition model before admission to
one beat**. An included row has a concrete initial state and a sequence of declared transition
source IDs reaching its complete state. The report states `runtime_route_verified: false` and
includes the target scene/beat. It does not invent a route through the game, a command
acknowledgement, or a runtime save. A downstream replay tool must validate those missing control
semantics or report them unsupported.

The policy JSON declares complete typed initial states, invariants, quest-to-enum variable bindings,
world-event effects, whether external transitions remain open, and limits, including a total retained witness-step budget. Quest bindings require
an enum matching the quest stages and its declared initial stage. World events contain no effects
in the source model: effects must be supplied explicitly in the policy. Descriptive prose never
supplies an effect or invariant.

The shared checked condition/Set/Add evaluator is also used by the deterministic runtime. Analysis
explores concrete states, including right-hand-side variable reads and every bounded integer value;
it does not use representative interval samples as an exclusion proof. State variables read by the
beat's conditions and potential context determine displayed distinctions. Other state remains in
complete witnesses and exploration when needed for transitions.

A closed, exhausted model can exclude configurations it never reaches. Relevant host-owned state,
open external transitions, unmodelled authored effects or commands, missing quest bindings, and
unfinished time/state frontiers prevent exclusion. Scene admission failures remain unknown without
a runtime route proof. Unknown rows stay visible and are never described as unreachable; this
version materializes only rows with verified model witnesses. Authors can supply additional explicit
initial states or transition bindings and rerun analysis. Display limits summarize undisplayed rows
without changing an already completed proof. Compilation refuses incomplete coverage of known
witnesses if the display limit omitted them.

## Priority, coverage and source protection

The compiler uses the same configured policies in validation, preview, generation and export.
An uncovered known witness fails compilation. Overlap is reported with the first matching authored
variant and any explicit fallback. Existing first-match ordering remains the runtime contract.
A graph with no configured analysis policy retains its existing compilation behavior.

Materialization refuses stale source/policy, unknown or excluded selections, identity collisions,
and any locked variant or slot whose priority would change. It inserts deterministic, disjoint
conditional placeholders before existing variants and preserves existing relative order and prose.
The shared editorial transaction records the structural event; shared undo/redo restores the exact
authorized structure. Development compilation permits missing placeholder wording as a warning,
so those real variant identities can be frozen. Release still requires prepared, reviewed content.

Requests bind canonical policy content and retain their immutable matrix report reference. Dispatch,
completion and acceptance recheck the binding. A newly configured policy also blocks an older
unbound request from being newly dispatched or automatically accepted. Historical request bytes remain readable.
Equivalent fresh reports retain semantic output reuse: fresh receipt IDs and filesystem timestamps
are not reuse keys. Policy stamps are used only for concurrent capture checks. World/state changes
invalidate matrix generation; target guards and frozen context protect source and wording changes.

## Twelve potential configurations, seven witnesses

The bundled example declares `permit` (boolean), `stage` (`arrival`, `hearing`, `departed`), and
`evidence` (integer 0–1): 2 × 3 × 2 = 12 configurations. Its explicit sequence is:

1. Start without a permit at arrival with no evidence.
2. Grant a permit.
3. Find evidence (`Add 1`).
4. Enter the hearing.
5. Revoke the permit.
6. Depart.
7. Restore the permit.

Invariants restrict non-arrival states to evidence 1 and an arrival without a permit to evidence 0.
The closed declared model therefore visits exactly seven configurations and excludes five. Those
numbers describe this fixture's supplied model, not arbitrary games or verified runtime routes.

Reproduce the native fixture from `src-tauri`:

```sh
mkdir -p /tmp/wobu-170-native-fixture
cargo run -p wobu-store --example variant_matrix_fixture -- /tmp/wobu-170-native-fixture
```

Open the generated project and its hearing scene, then **Variants… → Save policy and plan**. Expect
12 potential, 7 included, 5 excluded and 0 unknown. Choose the second line to materialize seven empty
Generated variants. Build then exposes seven separate requests, each with its witnessed state.
Native evidence and test outcomes are recorded separately in `docs/evidence/narrative-170`.
