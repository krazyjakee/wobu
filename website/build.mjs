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
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { renderPage, useMark } from './lib/layout.mjs'
import { renderMarkdown } from './lib/markdown.mjs'
import { escapeHtml, site } from './lib/site.mjs'
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
const GUIDE_BLOB_DIR = `${site.repo}/blob/main/docs/guide`

/**
 * The documents `docs/legal/` owns. They are optional inputs: a checkout that
 * does not have them yet still produces a complete, publishable site.
 */
const LEGAL_DOCUMENTS = [
  {
    slug: 'privacy-policy',
    source: 'docs/legal/privacy-policy.md',
    label: 'Privacy policy',
    summary: `Everywhere your work can possibly go, and what is sent when it does. What stays on
      your disk, where it sits, and how your keys are kept.`,
    description:
      "Wobu's privacy policy: no servers, no account, no tracking, and a plain list of the only " +
      'times the app talks to anything at all.',
  },
  {
    slug: 'terms',
    source: 'docs/legal/terms.md',
    label: 'Terms of use',
    summary: `What you are allowed to do with Wobu, the fact that it comes with no warranty, and
      why who owns a generated picture is between you and the AI service, not us.`,
    description:
      "Wobu's terms of use: what you may do with the app, and what it does not promise you.",
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
    summary: 'The licence Wobu is given to you under, in full. Short, and unusually readable.',
    description: 'The MIT licence Wobu is released under.',
  }
  documents.push({ ...licenceDocument, available: licence !== null })
  pages.push(licence === null ? legalStubPage(licenceDocument) : licencePage(licence))

  pages.push(legalIndexPage(documents))
  return pages
}

/**
 * The product guide is Markdown in `docs/guide/`, and the same files are
 * compiled into the application (issue #132) — so it is rendered here rather
 * than maintained twice. `contents.json` supplies the running order; a sibling
 * `*.md` link resolves to the published page, and a link into `docs/*.md`
 * points at GitHub rather than 404ing.
 *
 * Guide pages link back into the app with a `wobu:` scheme — "open the
 * shortcuts reference" — which means nothing in a browser. Those links are
 * flattened to their own text so the sentence still reads.
 */
function guideContents(pages, currentSlug) {
  const items = pages
    .map((page) => {
      const current = page.slug === currentSlug ? ' aria-current="page"' : ''
      return `<li><a href="${page.slug}.html"${current}>${escapeHtml(page.title)}</a></li>`
    })
    .join('\n            ')

  return `<nav class="toc" aria-labelledby="toc-heading">
          <h2 id="toc-heading">Guide</h2>
          <ol>
            ${items}
          </ol>
        </nav>`
}

function guideSteps(previous, next) {
  const steps = []
  if (previous) {
    steps.push(
      `<a class="text-link" href="${previous.slug}.html">← ${escapeHtml(previous.title)}</a>`,
    )
  }
  if (next) {
    steps.push(`<a class="text-link" href="${next.slug}.html">${escapeHtml(next.title)} →</a>`)
  }
  if (steps.length === 0) return ''
  return `<p class="doc-source">${steps.join(' &nbsp;·&nbsp; ')}</p>`
}

async function buildGuide() {
  const contentsPath = join(repoRoot, 'docs/guide/contents.json')
  if (!existsSync(contentsPath)) {
    console.warn('  ! docs/guide is absent — the site will link to a guide that is not there')
    return []
  }

  const contents = JSON.parse(await readFile(contentsPath, 'utf8'))
  const ordered = contents.groups.flatMap((group) => group.pages)
  const pages = []

  for (const [index, entry] of ordered.entries()) {
    const markdown = await readIfPresent(`docs/guide/${entry.slug}.md`)
    if (markdown === null) {
      console.warn(`  ! docs/guide/${entry.slug}.md is listed in contents.json but absent`)
      continue
    }

    const { title, html } = renderMarkdown(markdown, { sourceBlobDir: GUIDE_BLOB_DIR })
    const body = html
      .replace(/<a href="wobu:[^"]*">([\s\S]*?)<\/a>/g, '$1')
      // A definition table is written with the empty header row GFM demands.
      // The app's renderer drops it; so does this one, or the page carries a
      // blank ruled band above every such table.
      .replace(/<thead>\s*<tr>(?:\s*<th>\s*<\/th>)+\s*<\/tr>\s*<\/thead>/g, '')

    const main = `      <div class="wrap doc-wrap">
        ${guideContents(ordered, entry.slug)}
        <article class="doc">
${body}
          <hr />
          ${guideSteps(ordered[index - 1], ordered[index + 1])}
          <p class="doc-source">
            This page comes from
            <a href="${GUIDE_BLOB_DIR}/${entry.slug}.md" rel="noopener">a file in the source
            code</a> — the very same one the app shows you in its own built-in guide.
          </p>
        </article>
      </div>`

    pages.push({
      path: `guide/${entry.slug}.html`,
      nav: 'guide',
      depth: 1,
      title: entry.slug === 'index' ? title : entry.title,
      description: entry.summary,
      main,
      bodyClass: 'doc-page',
    })
  }

  return pages
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
  const pages = [homePage(), downloadPage(), ...(await buildLegal()), ...(await buildGuide())]
  const written = []
  for (const page of pages) {
    written.push(await writePage(page))
  }

  await writeSitemap(written)

  console.log(`Built ${written.length} pages into ${outDir}`)
  for (const path of written) console.log(`  · ${path}`)
  console.log(branded ? '  · branding/ artwork in use' : '  · built-in fallback mark in use')
}

await main()
