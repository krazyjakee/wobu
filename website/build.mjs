/**
 * Builds the wobu.app static site into `website/dist`.
 *
 * No framework and no client-side JavaScript: the output is HTML, one
 * stylesheet, and a copy of the product guide that already lives in `docs/`.
 * Nothing outside `website/` is written to — repository documents are inputs.
 *
 *   node build.mjs            build into ./dist
 *   node build.mjs --out DIR  build somewhere else
 *   node build.mjs --root DIR read repository inputs from somewhere else
 */
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { renderPage, useMark } from './lib/layout.mjs'
import { renderMarkdown } from './lib/markdown.mjs'
import { site } from './lib/site.mjs'
import { downloadPage } from './pages/download.mjs'
import { homePage } from './pages/home.mjs'
import { legalDocumentPage, legalIndexPage, legalStubPage, licencePage } from './pages/legal.mjs'

const here = dirname(fileURLToPath(import.meta.url))

function flag(name, fallback) {
  const index = process.argv.indexOf(`--${name}`)
  return index === -1 ? fallback : resolve(process.argv[index + 1])
}

const repoRoot = flag('root', resolve(here, '..'))
const outDir = flag('out', join(here, 'dist'))

const LEGAL_BLOB_DIR = `${site.repo}/blob/main/docs/legal`

/**
 * The documents `docs/legal/` owns. They are optional inputs: a checkout that
 * does not have them yet still produces a complete, publishable site.
 */
const LEGAL_DOCUMENTS = [
  {
    slug: 'privacy-policy',
    source: 'docs/legal/privacy-policy.md',
    label: 'Privacy policy',
    summary: `Every outbound destination and what is sent to it, what stays on disk and where, and
      how credentials are held in the operating-system keychain.`,
    description:
      "Wobu's privacy policy: no servers, no account, no telemetry, and an exact account of the " +
      'only requests the application ever makes.',
  },
  {
    slug: 'terms',
    source: 'docs/legal/terms.md',
    label: 'Terms of use and EULA',
    summary: `The MIT grant restated for a downloaded binary, the absence of warranty, and how
      provider terms and generated-content ownership pass through to your own agreement with each
      provider.`,
    description:
      "Wobu's terms of use and end user licence agreement, restating the MIT grant for a " +
      'downloaded binary.',
  },
]

function canonicalFor(path) {
  const clean = path === 'index.html' ? '' : path
  return `${site.url}/${clean}`
}

async function writePage(page) {
  const target = join(outDir, page.path)
  await mkdir(dirname(target), { recursive: true })
  await writeFile(target, renderPage({ ...page, canonical: canonicalFor(page.path) }), 'utf8')
  return page.path
}

async function readIfPresent(relativePath) {
  const absolute = join(repoRoot, relativePath)
  if (!existsSync(absolute)) return null
  return readFile(absolute, 'utf8')
}

async function buildLegal() {
  const pages = []
  const documents = []

  for (const document of LEGAL_DOCUMENTS) {
    const markdown = await readIfPresent(document.source)

    if (markdown === null) {
      console.warn(
        `  ! ${document.source} is absent — publishing a stub for /legal/${document.slug}.html`,
      )
      documents.push({ ...document, available: false })
      pages.push(legalStubPage(document))
      continue
    }

    const { title, html, toc } = renderMarkdown(markdown, { sourceBlobDir: LEGAL_BLOB_DIR })
    documents.push({ ...document, available: true })
    pages.push(
      legalDocumentPage({
        ...document,
        title,
        html,
        toc,
        sourceUrl: `${LEGAL_BLOB_DIR}/${document.source.split('/').pop()}`,
      }),
    )
  }

  const licence = await readIfPresent('LICENSE')
  const licenceDocument = {
    slug: 'licence',
    label: 'MIT licence',
    summary: 'The licence Wobu itself is released under, in full.',
    description: 'The MIT licence Wobu is released under.',
  }
  documents.push({ ...licenceDocument, available: licence !== null })
  pages.push(licence === null ? legalStubPage(licenceDocument) : licencePage(licence))

  pages.push(legalIndexPage(documents))
  return pages
}

/**
 * The product guide is already a static site, so it is published as-is. Its
 * only site-hostile links are the ones into `docs/*.md`, which exist one
 * directory up in the repository and nowhere at all here; those are pointed at
 * GitHub rather than left to 404.
 */
async function copyGuide() {
  const guide = join(repoRoot, 'docs/guide')
  if (!existsSync(guide)) {
    console.warn('  ! docs/guide is absent — the site will link to a guide that is not there')
    return false
  }

  const target = join(outDir, 'guide')
  await cp(guide, target, { recursive: true })

  for (const entry of await readdir(target, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith('.html')) continue
    const file = join(target, entry.name)
    const source = await readFile(file, 'utf8')
    const rewritten = source.replace(
      /href="\.\.\/([\w./-]+\.md)(#[^"]*)?"/g,
      (_match, path, hash = '') => `href="${site.repo}/blob/main/docs/${path}${hash}"`,
    )
    if (rewritten !== source) await writeFile(file, rewritten, 'utf8')
  }

  return true
}

/**
 * `branding/` holds the master artwork every app icon is generated from. When
 * it is in the checkout the site uses it directly, so the website, the
 * installer icon and the app can never drift apart; when it is not, the
 * fallbacks in `lib/layout.mjs` and `static/favicon.svg` stand in.
 */
async function useRepositoryBranding() {
  const icon = await readIfPresent('branding/wobu-icon.svg')
  if (icon !== null) await writeFile(join(outDir, 'favicon.svg'), icon, 'utf8')

  const markSource = await readIfPresent('branding/wobu-mark.svg')
  if (markSource === null) return icon !== null

  // Drop the XML prologue and any pixel width/height so CSS sizes the inline
  // copy, and hide it from assistive technology — every use has a text label.
  const body = markSource.slice(markSource.indexOf('<svg'))
  useMark(
    body
      .replace(/\s(width|height)="[^"]*"/g, '')
      .replace('<svg', '<svg class="mark" aria-hidden="true" focusable="false"'),
  )
  return true
}

async function writeSitemap(paths) {
  const urls = paths
    .map((path) => `  <url><loc>${canonicalFor(path)}</loc></url>`)
    .sort()
    .join('\n')

  await writeFile(
    join(outDir, 'sitemap.xml'),
    `<?xml version="1.0" encoding="UTF-8"?>\n` +
      `<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`,
    'utf8',
  )
}

async function main() {
  await rm(outDir, { recursive: true, force: true })
  await mkdir(outDir, { recursive: true })

  // `dotfiles` are not special to `cp`, so `.nojekyll` comes along with the
  // rest of `static/` — which it must, or GitHub Pages hides `_`-prefixed paths.
  await cp(join(here, 'static'), outDir, { recursive: true })

  const branded = await useRepositoryBranding()
  const pages = [homePage(), downloadPage(), ...(await buildLegal())]
  const written = []
  for (const page of pages) {
    written.push(await writePage(page))
  }

  const guide = await copyGuide()
  await writeSitemap(written)

  console.log(`Built ${written.length} pages into ${outDir}`)
  for (const path of written) console.log(`  · ${path}`)
  if (guide) console.log('  · guide/ (copied from docs/guide)')
  console.log(branded ? '  · branding/ artwork in use' : '  · built-in fallback mark in use')
}

await main()
