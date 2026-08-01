/**
 * Three deliberate departures from the defaults, all of them just writing down
 * how `src/` was already written: no semicolons, single quotes, and a 100-column
 * line — the same width `src-tauri/rustfmt.toml` sets, so the two halves of the
 * codebase wrap at the same place in a side-by-side diff.
 *
 * @type {import('prettier').Config}
 */
export default {
  semi: false,
  singleQuote: true,
  printWidth: 100,
}
