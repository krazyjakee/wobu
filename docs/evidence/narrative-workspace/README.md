# Native Ashfall preview evidence

Recorded 8 September 2026 in the actual Linux Tauri/WebKitGTK application on an isolated
Xephyr display. The application window is 1440 × 900 at 100% UI scale. The frontend includes
the Flow fixes documented in [the focused Flow evidence](../narrative-186/README.md); the
backend is the baseline `2159eaa` development binary. These checks exercise the existing
Ashfall runtime and do not establish acceptance of the new supporting-text or dependency code.

The [60-second recording](ashfall-preview-native.mp4) uses the original Ashfall council
fixture created earlier through the product's example entry point and saved to a temporary
project. A temporary harness focuses actual DOM controls, inspects results, and changes the
product theme setting. Trusted OS keyboard input enters values and activates controls.
This is a harness-assisted keyboard walkthrough, not an uninterrupted Tab-only navigation claim.
All compilation and playback use the real Tauri bridge. No IPC replies or provider outputs are
mocked; the explicitly simulated host result is the Preview product's normal host-command UI.

| Starting inputs | Action | Observed result |
| --- | --- | --- |
| Trust 40, knows logbook | Present the logbook; acknowledge `ashfall_file_record` with `ashfall_record_filed = true` | Trust 50, record filed, council accepts the record and starts an investigation |
| Trust 20, knows logbook | Ask for a fair hearing | Logbook choice unavailable; trust 25, clerk asks for the logbook |
| Trust 40, does not know logbook | Challenge the council's silence | Logbook choice unavailable; trust 30, council asks for a witness |

Every route reconverged on Council response and then ended at Hearing complete. The adjacent
JSON snapshots retain the actual state, source selection and DOM error list (empty in each).
The [logbook](ashfall-route-logbook-dark.png), [fair hearing](ashfall-route-fair-dark.png) and
[challenge](ashfall-route-challenge-light.png) screenshots show the different prepared responses
in both themes. Release-readiness warnings are expected for the unapproved example wording.

The real-file public-command tests in `commands/narrative/ashfall_tests.rs` separately verify
save/reopen identity and unchanged source across all three configurations. This recording
covers playback; it does not show project reopening or establish the large-library budgets,
World backlinks, conflict handling, or cross-platform accessibility acceptance in #194.
