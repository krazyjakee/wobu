# Native supporting-text authoring, review and invalidation

Verified 8 September 2026 using the real Tauri/WebKitGTK application in an isolated Xephyr
window, 1440 × 900 at 100% scale. The backend includes the combined supporting-text and
precise-dependency implementation through `3087666`; frontend fixes include `c79a09f`.
The deliberately created Harbor Voices project uses the checked-in fixture generator,
including three actual character nodes and the linked quest. No IPC response is mocked.

The temporary harness focuses product controls; trusted OS keyboard input opens each asset,
inspects context, approves and locks the first bark, opens Review, and checks Release export.
Theme changes, the voice mutation and bulk test approvals use the product settings store or
public Tauri commands. This is harness-assisted native validation, not a Tab-only walkthrough.
No provider or engine adapter is called. These are handwritten test fixtures, and all review
decisions below apply only to that temporary project.

- All six kinds opened with their actual source, conditions and identities:
  [bark](text-bark-dark.png), [ambient exchange](text-ambient-lines-light.png),
  [reaction](text-reaction-light.png), [codex](text-codex-light.png),
  [quest summary](text-quest-light.png), and [journal](text-journal-light.png).
- The bark's [frozen generation context](text-bark-context-dark.png) resolves the actual
  keeper voice and linked quest. The shared Review UI [approved and locked its first line](text-reviewed-locked-dark.png).
- A public `node_upsert` changed only the keeper's narrative voice. The affected set was
  exactly the four keeper bark variants, with no unrelated asset. Every body and wording
  revision stayed equal, the editorial head stayed equal, and the lock remained in place.
  [Raw before/after source and affected results](text-voice-invalidation.json) retain that check.
  Review [withdrew readiness](text-review-stale-light.png) while retaining protected wording.
- The [Why affected inspector](text-why-affected-light.png) names the exact voice field and
  selected text slot. After test attestation/export, restoring the original voice reproduces
  the same four-line invalidation; this second capture makes the explanation visible.
- Release preflight initially refused the unapproved/outdated fixture. Fresh public review
  requests approved the test wording or attested the protected bark, using each current
  guard/context. Release then reported [0 scenes, 20 strings and 8,637 bytes](text-release-ready-light.png),
  with zero diagnostics, and the public export command successfully published the package.
  [The complete preflight, decisions and export report](text-native-export.json) retain its hash.
  The later voice-restoration capture deliberately makes the temporary project stale again;
  it does not alter the previously exported package.

Adjacent DOM snapshots report no browser errors. Runtime delivery, ambient speaker order,
seeded bark repetition/save state, text-only restore and Release rejection regressions run
in Rust tests. Generation proposal tests use mocked providers through the actual queue and
acceptance pipeline; no live-model quality or N5 engine-conformance claim follows from these
screenshots. Large-project performance remains tracked separately in #182/#194.
