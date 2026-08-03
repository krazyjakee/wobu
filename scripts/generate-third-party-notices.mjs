#!/usr/bin/env node
/**
 * Generates `THIRD-PARTY-NOTICES.md` from the two lockfiles that actually
 * decide what ships: `package-lock.json` and `src-tauri/Cargo.lock`.
 *
 * The package *set* comes from the lockfiles rather than from `node_modules`
 * or a dependency walk, because that is the only description of the tree that
 * is identical on every machine. The licence *texts* come from the already
 * unpacked copies on disk — `node_modules/<pkg>/LICENSE` and the cargo
 * registry's source directories — so this needs no network of its own and adds
 * no dependency to the repo.
 *
 * Determinism is the whole point: CI regenerates the file and fails on any
 * difference, so anything that varies between two checkouts of the same commit
 * would be a permanently red check. Two things vary and are handled here:
 *
 *   - npm ships one platform's binaries per install. Entries the lockfile marks
 *     with `os`/`cpu` are listed from lockfile metadata only, never from files
 *     that exist on Linux and not on macOS.
 *   - `cargo metadata` is asked for the whole graph, not this host's slice, and
 *     is run with `--locked` so it cannot quietly re-resolve.
 *
 * Usage:
 *   node scripts/generate-third-party-notices.mjs           # write the file
 *   node scripts/generate-third-party-notices.mjs --check   # fail if stale
 *
 * Either mode exits non-zero if a licence in the distributed graph is one Wobu
 * cannot ship under (copyleft or non-commercial); see LEVEL below.
 */

import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const OUTPUT = join(ROOT, 'THIRD-PARTY-NOTICES.md')

/* ── licence classification ───────────────────────────────────────────────── */

/**
 * What a licence costs us, not how it is categorised in the abstract.
 *
 * `OK` is anything that asks only for attribution — satisfied by the file this
 * script writes. `REVIEW` is file-level or weak copyleft: shipping an unmodified
 * upstream build is fine, patching one and not publishing the patch is not, so
 * it is listed for a human rather than blocked for a machine. `BLOCKED` is a
 * term Wobu cannot meet at all while shipping an MIT desktop app — a reciprocal
 * licence that reaches the whole binary, or one that forbids commercial use.
 */
export const LEVEL = { OK: 0, REVIEW: 1, BLOCKED: 2 }
const LEVEL_NAME = ['permissive', 'review', 'blocked']

/** Attribution-only. Matched exactly, after the aliasing below. */
const PERMISSIVE = new Set([
  '0BSD',
  'AFL-2.1',
  'Apache-1.1',
  'Apache-2.0',
  'Artistic-2.0',
  'BSD-1-Clause',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'BSD-4-Clause',
  'BSL-1.0',
  'BlueOak-1.0.0',
  'CC0-1.0',
  'CDLA-Permissive-1.0',
  'CDLA-Permissive-2.0',
  'ISC',
  'MIT',
  'MIT-0',
  'MITNFA',
  'NCSA',
  'OpenSSL',
  'PSF-2.0',
  'Python-2.0',
  'Ruby',
  'Unicode-3.0',
  'Unicode-DFS-2016',
  'Unlicense',
  'WTFPL',
  'Zlib',
  'libpng-2.0',
])

/** Weak or file-level copyleft, plus the share-alike content licences. */
const REVIEW_PREFIXES = [
  'LGPL',
  'MPL',
  'EPL',
  'CDDL',
  'CECILL-C',
  'MS-RL',
  'CC-BY-SA',
  'CC-BY-4',
  'CC-BY-3',
  'CDLA-Sharing',
  'APSL',
  'IPL',
  'Sleepycat',
]

/** Reciprocal-to-the-binary, source-availability, or non-commercial. */
const BLOCKED_PREFIXES = [
  'GPL',
  'AGPL',
  'SSPL',
  'OSL',
  'EUPL',
  'CPAL',
  'RPL',
  'QPL',
  'CC-BY-NC',
  'CC-NC',
  'BUSL',
  'Elastic',
  'PolyForm',
  'Parity',
  'Commons-Clause',
  'JSON',
  'Hippocratic',
  'SISSL',
  'NPOSL',
]

/** Exceptions that put a linking carve-out on an otherwise viral licence. */
const LINKING_EXCEPTIONS = /classpath|llvm|linking|gcc|autoconf|bison|font/i

function levelOfId(id) {
  const bare = id.replace(/\+$/, '')
  if (PERMISSIVE.has(bare)) return LEVEL.OK
  const upper = bare.toUpperCase()
  // Review before blocked: the prefixes are tested with `startsWith`, and
  // `LGPL-2.1` must not be read as a `GPL` that happens to be spelled oddly.
  if (REVIEW_PREFIXES.some((p) => upper.startsWith(p.toUpperCase()))) return LEVEL.REVIEW
  if (BLOCKED_PREFIXES.some((p) => upper.startsWith(p.toUpperCase()))) return LEVEL.BLOCKED
  // An id nobody here recognises is a question, not a pass.
  return LEVEL.REVIEW
}

/**
 * The level of a whole SPDX expression.
 *
 * `OR` takes the cheapest branch and `AND` the dearest, which is what makes
 * `MIT OR Apache-2.0 OR LGPL-2.1-or-later` permissive rather than a finding: we
 * choose MIT and the LGPL term never applies. Doing this by substring match, as
 * a naive scan would, reports a third of the crates.io graph as copyleft.
 */
export function levelOfExpression(expression) {
  if (!expression) return LEVEL.REVIEW
  // `MIT/Apache-2.0` is pre-SPDX cargo syntax for a disjunction, still on
  // several dozen crates in this graph.
  const normalised = expression.replace(/\s*\/\s*/g, ' OR ')
  const tokens = normalised.match(/\(|\)|[^\s()]+/g) ?? []
  let at = 0
  const peek = () => tokens[at]
  const next = () => tokens[at++]

  function factor() {
    if (peek() === '(') {
      next()
      const value = expr()
      if (peek() === ')') next()
      return value
    }
    const id = next() ?? ''
    if (peek() && peek().toUpperCase() === 'WITH') {
      next()
      const exception = next() ?? ''
      const base = levelOfId(id)
      // A linking exception is exactly the thing that makes GPL-family code
      // shippable inside a binary that is not itself GPL.
      if (base === LEVEL.BLOCKED && LINKING_EXCEPTIONS.test(exception)) return LEVEL.REVIEW
      return base
    }
    return levelOfId(id)
  }

  function term() {
    let value = factor()
    while (peek() && peek().toUpperCase() === 'AND') {
      next()
      value = Math.max(value, factor())
    }
    return value
  }

  function expr() {
    let value = term()
    while (peek() && peek().toUpperCase() === 'OR') {
      next()
      value = Math.min(value, term())
    }
    return value
  }

  return tokens.length ? expr() : LEVEL.REVIEW
}

/* ── reading licence files off disk ───────────────────────────────────────── */

const LICENCE_FILE = /^(licen[cs]e|copying|copyright|notice|unlicen[cs]e)([-_. ].*)?$/i
/** Files that share the name but are code or metadata, not a notice. */
const NOT_TEXT = /\.(rs|js|mjs|cjs|ts|json|toml|yml|yaml|py|sh|lock|png|svg|gz)$/i
/** No licence is a megabyte. Anything that big is a vendored corpus. */
const MAX_TEXT_BYTES = 400 * 1024

/** Every notice-shaped file in a package's own directory, name-sorted. */
function licenceTexts(dir) {
  if (!dir || !existsSync(dir)) return []
  let names
  try {
    names = readdirSync(dir)
  } catch {
    return []
  }
  const out = []
  for (const name of names.sort()) {
    if (!LICENCE_FILE.test(name) || NOT_TEXT.test(name)) continue
    const path = join(dir, name)
    let stat
    try {
      stat = statSync(path)
    } catch {
      continue
    }
    if (!stat.isFile() || stat.size === 0 || stat.size > MAX_TEXT_BYTES) continue
    const text = normalise(readFileSync(path, 'utf8'))
    if (text) out.push({ name, text })
  }
  return out
}

/** CRLF and trailing whitespace differ between publishers, not between terms. */
function normalise(text) {
  return text
    .replace(/\r\n/g, '\n')
    .replace(/[ \t]+$/gm, '')
    .trim()
}

function digest(text) {
  return createHash('sha256').update(text).digest('hex')
}

/* ── the npm tree ─────────────────────────────────────────────────────────── */

function npmPackages() {
  const lock = JSON.parse(readFileSync(join(ROOT, 'package-lock.json'), 'utf8'))
  const packages = []
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    // The root entry is Wobu itself; a `link` entry is a workspace alias for a
    // directory already listed elsewhere.
    if (!path || entry.link) continue
    const name =
      entry.name ?? path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length)
    // Binaries published per platform: only one set is ever on disk, so their
    // texts are deliberately not read. See the note at the top of the file.
    const platformSpecific = Boolean(entry.os || entry.cpu)
    const dir = join(ROOT, path)
    packages.push({
      ecosystem: 'npm',
      name,
      version: entry.version ?? '0.0.0',
      licence: typeof entry.license === 'string' ? entry.license : null,
      distributed: !entry.dev,
      texts: platformSpecific ? [] : licenceTexts(dir),
      platformSpecific,
    })
  }
  return dedupeAndSort(packages)
}

/* ── the cargo tree ───────────────────────────────────────────────────────── */

/** `[[package]]` blocks, which is all of Cargo.lock's format this needs. */
function cargoLockPackages() {
  const lock = readFileSync(join(ROOT, 'src-tauri', 'Cargo.lock'), 'utf8')
  return lock
    .split(/^\[\[package\]\]$/m)
    .slice(1)
    .map((block) => ({
      name: /^name = "(.*)"$/m.exec(block)?.[1] ?? '',
      version: /^version = "(.*)"$/m.exec(block)?.[1] ?? '',
      // Absent for the path crates in this workspace, which are ours.
      source: /^source = "(.*)"$/m.exec(block)?.[1] ?? null,
    }))
    .filter((p) => p.name && p.source)
}

/**
 * Licence strings and unpacked source directories for the crates in the lock.
 *
 * `cargo metadata` rather than a guess at the registry layout: it is the only
 * thing that knows where a crate was unpacked, and running it also guarantees
 * the sources are there at all — a platform-specific crate that this host never
 * builds is in the lockfile but is not downloaded by an ordinary `cargo build`.
 */
function cargoMetadata() {
  const json = execFileSync(
    'cargo',
    ['metadata', '--format-version', '1', '--locked', '--manifest-path', 'Cargo.toml'],
    { cwd: join(ROOT, 'src-tauri'), encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
  )
  const meta = JSON.parse(json)
  const byId = new Map()
  for (const pkg of meta.packages ?? []) {
    byId.set(`${pkg.name}@${pkg.version}`, pkg)
  }
  return byId
}

function cargoPackages() {
  const locked = cargoLockPackages()
  let meta
  try {
    meta = cargoMetadata()
  } catch (error) {
    throw new Error(
      'could not run `cargo metadata` in src-tauri/ — cargo and network access are ' +
        'needed to read the crate licences.\n' +
        String(error.stderr || error.message || error),
    )
  }
  const missing = []
  const packages = []
  for (const { name, version } of locked) {
    const pkg = meta.get(`${name}@${version}`)
    if (!pkg) {
      missing.push(`${name} ${version}`)
      continue
    }
    const dir = pkg.manifest_path ? dirname(pkg.manifest_path) : null
    const texts = licenceTexts(dir)
    // `license-file` is what a crate sets instead of an SPDX id when its terms
    // are not on the SPDX list; the file itself is picked up above.
    const licence = pkg.license ?? (pkg.license_file ? `see ${pkg.license_file}` : null)
    packages.push({
      ecosystem: 'cargo',
      name,
      version,
      licence,
      distributed: true,
      texts,
      platformSpecific: false,
    })
  }
  if (missing.length) {
    throw new Error(
      `${missing.length} crate(s) in Cargo.lock are missing from cargo metadata:\n` +
        missing.join('\n'),
    )
  }
  return dedupeAndSort(packages)
}

/** One entry per name+version, ordered so the output never depends on walk order. */
function dedupeAndSort(packages) {
  const seen = new Map()
  for (const pkg of packages) {
    const key = `${pkg.name}@${pkg.version}`
    const previous = seen.get(key)
    // Two copies of one version differ only in whether that copy on disk
    // happened to carry the licence files; keep the one that did.
    if (!previous || (previous.texts.length === 0 && pkg.texts.length > 0)) seen.set(key, pkg)
  }
  return [...seen.values()].sort(
    (a, b) => a.name.localeCompare(b.name, 'en') || compareVersion(a.version, b.version),
  )
}

function compareVersion(a, b) {
  const pa = a.split(/[.+-]/)
  const pb = b.split(/[.+-]/)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? ''
    const y = pb[i] ?? ''
    const nx = Number(x)
    const ny = Number(y)
    if (Number.isInteger(nx) && Number.isInteger(ny) && x !== '' && y !== '') {
      if (nx !== ny) return nx - ny
    } else if (x !== y) {
      return x < y ? -1 : 1
    }
  }
  return 0
}

/* ── rendering ────────────────────────────────────────────────────────────── */

const HEADER = `# Third-party notices

Wobu is MIT licensed — see \`LICENSE\` for its own terms and copyright.

This file is the attribution notice for everything Wobu is built from or ships
alongside. It is generated; do not edit it by hand. Regenerate it with:

    npm run licences

The package lists come from \`package-lock.json\` and \`src-tauri/Cargo.lock\`, and
the licence texts are copied verbatim from the packages themselves. CI
regenerates this file and fails if it differs from the committed copy, so a
dependency cannot be added without its notice arriving with it.
`

const ASSETS = `## Bundled assets

- **Fonts.** None are redistributed. The interface asks for the host's own UI
  and monospace faces (\`ui-sans-serif\`, \`ui-monospace\` and the platform names
  beside them in \`src/styles/tokens.css\`), so no font file is in the installer.
- **Icons.** The sprite in \`src/components/IconSprite.tsx\` is drawn for Wobu.
  No icon set is vendored.
- **Model weights.** None are redistributed. Every image, mesh and text model
  Wobu can use runs at a provider you configure, or on a ComfyUI server you
  install and populate yourself; the weights are never in Wobu's installer and
  their licences are the ones you accepted where you obtained them.
`

function table(rows, headers) {
  const lines = [`| ${headers.join(' | ')} |`, `| ${headers.map(() => '---').join(' | ')} |`]
  for (const row of rows) lines.push(`| ${row.join(' | ')} |`)
  return lines.join('\n')
}

/** Markdown table cells cannot contain a bare pipe. */
function cell(value) {
  return String(value ?? '').replace(/\|/g, '\\|')
}

/**
 * A code fence long enough to hold the text.
 *
 * Two dozen licence files in this graph are markdown themselves and contain
 * their own fences — a fixed three backticks around those would close the block
 * in the middle of the licence and render the rest as prose.
 */
function fence(text) {
  const longest = Math.max(0, ...[...text.matchAll(/`+/g)].map((m) => m[0].length))
  const ticks = '`'.repeat(Math.max(3, longest + 1))
  return `${ticks}\n${text}\n${ticks}`
}

function render({ cargo, npmRuntime, npmBuild, findings }) {
  // Texts are numbered in the order they are first reached, over a package list
  // that is already sorted — so the numbering is a function of the lockfiles.
  const texts = []
  const indexByDigest = new Map()
  function refs(pkg) {
    const ids = []
    for (const { text } of pkg.texts) {
      const key = digest(text)
      let index = indexByDigest.get(key)
      if (index === undefined) {
        index = texts.length + 1
        indexByDigest.set(key, index)
        texts.push({ text, packages: [] })
      }
      const entry = texts[index - 1]
      const label = `${pkg.name} ${pkg.version}`
      if (!entry.packages.includes(label)) entry.packages.push(label)
      if (!ids.includes(index)) ids.push(index)
    }
    return ids
  }

  const cargoRows = cargo.map((p) => [
    cell(p.name),
    cell(p.version),
    cell(p.licence ?? '— not declared —'),
    refs(p)
      .map((i) => `[${i}]`)
      .join(' '),
  ])
  const runtimeRows = npmRuntime.map((p) => [
    cell(p.name),
    cell(p.version),
    cell(p.licence ?? '— not declared —'),
    refs(p)
      .map((i) => `[${i}]`)
      .join(' '),
  ])
  const buildRows = npmBuild.map((p) => [
    cell(p.name),
    cell(p.version),
    cell(p.licence ?? '— not declared —'),
  ])

  const tally = new Map()
  for (const p of [...cargo, ...npmRuntime]) {
    const key = p.licence ?? '— not declared —'
    tally.set(key, (tally.get(key) ?? 0) + 1)
  }
  const tallyRows = [...tally.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'en'))
    .map(([licence, count]) => [cell(licence), String(count)])

  const blocked = findings.filter((f) => f.level === LEVEL.BLOCKED)
  const review = findings.filter((f) => f.level === LEVEL.REVIEW)

  const parts = [
    HEADER,
    `## Summary

| | |
| --- | --- |
| Rust crates linked into the binary | ${cargo.length} |
| npm packages bundled into the frontend | ${npmRuntime.length} |
| npm packages used only to build and test Wobu | ${npmBuild.length} |
| Distinct licence texts reproduced below | ${texts.length} |

The build-and-test packages are listed for completeness. Their code is not in
the installer, so their notices are not reproduced in full.
`,
    `### Licences in the distributed graph

${table(tallyRows, ['Licence', 'Packages'])}
`,
    `### Licences needing a human

${
  blocked.length === 0 && review.length === 0
    ? 'Nothing in the distributed graph is copyleft or non-commercial.'
    : [
        blocked.length
          ? `**Incompatible — Wobu cannot ship these under MIT.**\n\n${table(
              blocked.map((f) => [cell(f.name), cell(f.version), cell(f.licence)]),
              ['Package', 'Version', 'Licence'],
            )}`
          : null,
        review.length
          ? `**Worth a look.** File-level or weak copyleft, or terms this script\ndoes not recognise. Shipping the upstream build unmodified is fine; patching\none of these and not publishing the patch is not.\n\n${table(
              review.map((f) => [cell(f.name), cell(f.version), cell(f.licence)]),
              ['Package', 'Version', 'Licence'],
            )}`
          : null,
      ]
        .filter(Boolean)
        .join('\n\n')
}
`,
    ASSETS,
    `## Rust crates linked into the binary

${table(cargoRows, ['Crate', 'Version', 'Licence', 'Texts'])}
`,
    `## npm packages bundled into the frontend

${table(runtimeRows, ['Package', 'Version', 'Licence', 'Texts'])}
`,
    `## npm packages used only to build and test Wobu

Not redistributed. Listed so the notice covers the whole lockfile.

${table(buildRows, ['Package', 'Version', 'Licence'])}
`,
    `## Licence texts

Each text appears once, with the packages that ship it. Where a package offers a
choice of licences, every text it ships is reproduced rather than only the one
Wobu relies on.
`,
  ]

  for (const [i, entry] of texts.entries()) {
    const shown = entry.packages.slice(0, 8).join(', ')
    const more = entry.packages.length > 8 ? `, and ${entry.packages.length - 8} more` : ''
    parts.push(`### [${i + 1}] ${shown}${more}\n\n${fence(entry.text)}\n`)
  }

  return `${parts.join('\n')}`
}

/* ── audit ────────────────────────────────────────────────────────────────── */

export function audit(packages) {
  const findings = []
  for (const pkg of packages) {
    const level = levelOfExpression(pkg.licence)
    if (level === LEVEL.OK) continue
    findings.push({
      name: pkg.name,
      version: pkg.version,
      licence: pkg.licence ?? '— not declared —',
      level,
    })
  }
  return findings.sort(
    (a, b) =>
      b.level - a.level || a.name.localeCompare(b.name, 'en') || (a.version < b.version ? -1 : 1),
  )
}

/* ── entry point ──────────────────────────────────────────────────────────── */

function main(argv) {
  const check = argv.includes('--check')

  const cargo = cargoPackages()
  const npm = npmPackages()
  const npmRuntime = npm.filter((p) => p.distributed)
  const npmBuild = npm.filter((p) => !p.distributed)
  const findings = audit([...cargo, ...npmRuntime])

  const content = render({ cargo, npmRuntime, npmBuild, findings })

  let failed = false
  if (check) {
    const existing = existsSync(OUTPUT) ? readFileSync(OUTPUT, 'utf8') : null
    if (existing !== content) {
      console.error(
        'THIRD-PARTY-NOTICES.md is out of date. Run `npm run licences` and commit the result.',
      )
      failed = true
    } else {
      console.log(`THIRD-PARTY-NOTICES.md is up to date (${cargo.length + npm.length} packages).`)
    }
  } else {
    writeFileSync(OUTPUT, content)
    console.log(
      `Wrote THIRD-PARTY-NOTICES.md: ${cargo.length} crates, ${npmRuntime.length} bundled npm ` +
        `packages, ${npmBuild.length} build-only npm packages.`,
    )
  }

  const blocked = findings.filter((f) => f.level === LEVEL.BLOCKED)
  const review = findings.filter((f) => f.level === LEVEL.REVIEW)
  for (const f of review) {
    console.warn(`review: ${f.name} ${f.version} — ${f.licence}`)
  }
  for (const f of blocked) {
    console.error(`incompatible: ${f.name} ${f.version} — ${f.licence} (${LEVEL_NAME[f.level]})`)
  }
  if (blocked.length) {
    console.error(
      `${blocked.length} package(s) carry a licence Wobu cannot ship under. Replace them or ` +
        'take a written decision before releasing.',
    )
    failed = true
  }

  process.exitCode = failed ? 1 : 0
}

// Importable as well as runnable: the classifier is unit-tested next door, and
// running `main` on import would make that test regenerate the notices.
const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (invoked) main(process.argv.slice(2))
