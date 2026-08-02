# Repository Guidelines

## Project Structure & Module Organization

Wobu is a Tauri 2 desktop application. The React/TypeScript frontend lives in `src/`: components are
in `components/`, shared logic in `lib/`, stores in `store/`, hooks in `hooks/`, and CSS in `styles/`.
Frontend tests sit beside code as `*.test.ts(x)`; shared setup is in `src/test/`.

The Rust workspace is under `src-tauri/`. Tauri command glue belongs in `src-tauri/src/`; domain
crates in `src-tauri/crates/` cover the model, storage, influence resolution, providers, jobs, and
sync. Integration tests live in each crate's `tests/`. Documentation is in `docs/`, and `examples/`
contains sample projects.

## Build, Test, and Development Commands

- `npm ci`: install the locked Node dependencies (Node 22 is used in CI).
- `npm run tauri dev`: run the complete desktop app with the Rust backend and Vite hot reload.
- `npm run build`: type-check and create frontend assets in `dist/`.
- `npm run check`: run TypeScript checks, ESLint, Prettier verification, and Vitest once.
- `npm run check:code-health`: detect dead code and excessive duplication.
- `cd src-tauri && cargo test --workspace`: run all Rust tests with the pinned toolchain.
- `cd src-tauri && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`:
  reproduce the Rust formatting and lint gates.

## Coding Style & Naming Conventions

Prettier enforces single quotes, no semicolons, and 100-column lines. Use two-space TypeScript
indentation, `PascalCase` components, `camelCase` functions/hooks, and `_` for intentionally unused
bindings. Import Vitest APIs explicitly. Rust uses edition 2024, `rustfmt` at 100 columns,
`snake_case` functions, and `PascalCase` types; unsafe code is denied.

## Testing Guidelines

Vitest runs in jsdom and matches `src/**/*.test.{ts,tsx}`. Add focused tests beside changed code and
cross-module Rust tests to the relevant crate. No coverage threshold is configured; cover
regressions and error paths.

## Commit & Pull Request Guidelines

History uses short, imperative, sentence-case subjects such as `Add peer-to-peer sharing flow
(#109)`. Keep commits scoped and include issue numbers when applicable. Pull requests should explain
the user-visible result, list verification performed, link relevant issues, and include screenshots
or recordings for UI changes. Update `docs/` when behavior, configuration, or roadmap status changes.

## Security & Configuration

Copy `.env.example` to `.env` only for development. Never commit credentials or place them inside a
shared `.wobu` project; release builds use the OS keychain. Treat generated project assets and local
indexes as user data, not source fixtures, unless deliberately adding an example.
