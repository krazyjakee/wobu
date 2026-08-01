# Code-health regression checks

The repository keeps dead code and copy/paste growth visible with three small,
repeatable checks. They are regression gates, not substitutes for TypeScript,
Clippy, or the test suites.

## Local commands

Install the pinned Node tools from the lockfile, then run both frontend/source
checks:

```sh
npm ci
npm run check:code-health
```

The individual commands are `npm run check:dead-code` and
`npm run check:duplicates`. Run `npm run check:duplicates:report` when you need
the full list of matching file pairs behind the concise regression result.

Install the pinned Rust direct-dependency checker once, then exercise both the
dependency graph and every crate's public rustdoc surface:

```sh
cargo install cargo-machete --version 0.9.2 --locked
cd src-tauri
cargo machete
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`cargo-machete` checks direct dependencies on stable Rust. Building workspace
documentation checks that every exported crate surface remains compilable and
promotes rustdoc warnings—including broken public links—to errors. Rust cannot
generally prove that a public item has no external consumer, so this does not
claim to be an unused-public-API oracle; public re-exports still require review
when their last workspace use is removed.

## What Knip considers unused

`knip.json` checks source files, runtime and development dependencies, runtime
exports, and exported types. Entry points and tool configuration are discovered
by Knip's Vite, Vitest, and package-script plugins. An export referenced inside
its declaring module is treated as module-internal rather than unused; a new
export with no reference, a new orphan file, or an unused package still fails.
There are no source-directory ignore globs in the Knip configuration.

## Duplication baseline

`.jscpd.json` detects clone blocks at least eight lines and 80 tokens long in
TypeScript, TSX, and Rust production source. Tests, fixtures, provider workflow
templates, and generated output are intentionally excluded: repetition in those
files is often the contract being expressed, not production logic that can
drift.

`.jscpd-baseline.json` records clone-block and duplicated-line counts per
language while the open refactor issues remove existing clusters. The wrapper
fails when either count rises, but accepts reductions. When a refactor lowers a
count, lower the committed baseline in the same change so later work cannot
regrow into the old allowance. Raising the baseline merely to make CI green is
not an acceptable fix.

## Proving the gates fail

The checks were established by first running them against the deliberately
unused code they found in the current tree: Knip rejected an orphaned component
and two unused query hooks, and cargo-machete rejected three unused direct
dependencies. Those findings were removed before enabling the gates.

To verify a tool upgrade without leaving the tree broken, make one temporary
change at a time and restore it immediately after observing a non-zero exit:

1. Add a `.ts` file under `src/` containing one unreferenced exported constant;
   `npm run check:dead-code` must report both the file and export.
2. Add an unused direct dependency to any workspace crate;
   `cargo machete` must name that manifest and dependency.
3. Add two production `.ts` files with the same block of at least eight lines
   and 80 tokens; `npm run check:duplicates` must report a baseline regression.

Never commit the probes or an increased baseline produced by them.
