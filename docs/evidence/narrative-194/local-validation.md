# Local authoring integration checkpoint

Validated implementation: `547522743bd61a427481014c24c2a98f6a0340d5`, 8 September 2026.
This checkpoint covers local implementation for #154, #186 and #194. These issues remain open;
N5 (#173–177) is excluded.

- Frontend: `npm run check` passed, including 1,250 tests in 135 files, TypeScript, ESLint and
  Prettier. Production build and packaged source-map verification passed.
- Code health: no unused declarations; 21 Rust clone blocks / 276 duplicated lines, zero
  TypeScript clone blocks. All remain within the existing budgets.
- Rust: formatting, all-target workspace Clippy with warnings denied, rustdoc with warnings
  denied, and cargo-machete passed. The nine real-filesystem Library regressions and the new
  Ashfall save/reopen/public Preview walkthrough passed.
- Full Rust workspace attempt: 1,679 tests passed and five existing tests were ignored;
  92 network tests failed because socket creation or binding was denied in this session.
  This run preceded the final Ashfall test and additional Library regression, which were
  checked separately. The full workspace gate is therefore not recorded as passed.
- Licence verification passed for all 1,161 packages using the unchanged verifier with real
  offline Cargo output captured through temporary files. Node's ordinary captured subprocess
  pipes report `EPERM` here. The temporary preload changes only that output transport and
  preserves the exact command, arguments, working directory, exit checks and output limit.
  No metadata fixture or filtered package graph was substituted. The normal npm command still
  cannot run with its default pipe transport; dependencies and notices were not changed.

Tests were not disabled or weakened to accommodate the environment. Rust checks used a separate
writable target directory and private XDG data directories; canonical project files remained
the authority.

## Remaining acceptance

The current session cannot connect to the native test display or create a loopback socket.
The complete native keyboard workflow, both themes, high scaling, narrow-window and conflict
states, and the walkthrough recording remain unverified. The 1,000-scene / 50,000-slot native
UI measurements for search, filtering, opening and returning also remain outstanding.

The [existing native IPC measurements](README.md) cover backend commands at their recorded
checkpoint. They do not establish current UI latency, geometry, focus behavior or memory usage.
Earlier mock screenshots are identified as such in the relevant guides.

GitHub reads were intermittent during local validation. This checkpoint does not establish
remote CI, merge readiness, or completed issue acceptance. Those require separate verification.
