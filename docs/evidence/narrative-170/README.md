# Bounded variant matrix native evidence

Captured on 2026-09-08 in the Linux Tauri/WebKit desktop at UI scale 1, dark theme. These are native
window screenshots, not browser mocks. The fixture is reproducible with the
[`variant_matrix_fixture` example](../../../src-tauri/crates/wobu-store/examples/variant_matrix_fixture.rs).
The local acceptance driver used by `native_proof.py` belongs to the temporary native harness; it
is not a production command surface.

- [Matrix and witness](matrix.png): 12 potential, 7 included, 5 excluded, 0 unknown; concrete state,
  source transition IDs, authored first-match/fallback evidence, and explicit unverified runtime
  route status.
- [Reopened matrix](reopened.png): the same canonical report ID after closing and reopening the UI.
  Planning preserved the scene file hashes.
- [960 × 620 window](matrix-960.png) and [scrolled actions](matrix-960-actions.png): the sheet fits
  the supported minimum window and its lower controls remain accessible through scrolling.
- [Shared Build](build.png): seven Generated targets, seven distinct complete witness states,
  zero attempts and zero dispatched requests. Existing wording, identity, provenance and policy
  were preserved. The desktop was restored to 1440 × 900 and the modal closed before handoff.

[Compact results](result.json) retain counts, assumptions, witnesses, selected targets/states and
native geometry. The walkthrough finished without frontend errors. It made no provider call and
performed no runtime route replay. Included states are witnesses under the explicitly declared
model; this is not a claim that seven runtime routes were executed.

Automated verification separately covers bounded exploration, RHS reads/integer steps, Unknown
frontiers and retained-witness limits, overlap/gap compilation, policy races, exact structural
materialization, shared undo/redo, portable receipts after index deletion/copy, equivalent-report
reuse, and retained synthetic output after a policy change. Synthetic output publication tests
exercise the existing receipt/acceptance path; they are not live provider validation. Focused
frontend tests cover selection, pagination, read-only/stale/error behavior and the shared Build
handoff. The integrated workspace tests, Clippy, frontend and code-health gates passed.
