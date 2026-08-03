import { marked } from 'marked'

import { escapeHtml } from './site.mjs'

marked.setOptions({ gfm: true, breaks: false })

function slug(text) {
  return text
    .toLowerCase()
    .replace(/<[^>]+>/g, '')
    .replace(/&[a-z]+;/g, '')
    .replace(/[^\w\- ]+/g, '')
    .trim()
    .replace(/\s+/g, '-')
}

/**
 * Documents in `docs/` link to each other with repository-relative paths. On
 * the site, a sibling `*.md` is a page we also publish, and anything else is
 * only meaningful on GitHub — so it is rewritten to a blob URL rather than
 * silently 404ing.
 */
function rewriteHref(rawHref, sourceBlobDir) {
  if (!rawHref || /^(https?:|mailto:|#|\/\/)/.test(rawHref)) return rawHref

  const [path, hash = ''] = rawHref.split('#')
  const fragment = hash ? `#${hash}` : ''

  if (!path) return rawHref
  if (!path.includes('/') && path.endsWith('.md')) {
    return `${path.slice(0, -3)}.html${fragment}`
  }

  const resolved = new URL(path, `${sourceBlobDir}/`).href
  return `${resolved}${fragment}`
}

/**
 * Renders one Markdown document to a page body.
 *
 * @param {string} markdown
 * @param {object} options
 * @param {string} options.sourceBlobDir Absolute GitHub URL of the directory
 *   the document lives in, used to resolve links we do not publish ourselves.
 * @returns {{ title: string, html: string, toc: Array<{ id: string, text: string }> }}
 */
export function renderMarkdown(markdown, { sourceBlobDir }) {
  const tokens = marked.lexer(markdown)

  marked.walkTokens(tokens, (token) => {
    if (token.type === 'link' || token.type === 'image') {
      token.href = rewriteHref(token.href, sourceBlobDir)
    }
  })

  const heading = tokens.find((token) => token.type === 'heading' && token.depth === 1)
  const title = heading ? heading.text.replace(/\s+/g, ' ').trim() : 'Wobu'

  const toc = tokens
    .filter((token) => token.type === 'heading' && token.depth === 2)
    .map((token) => ({ id: slug(token.text), text: token.text }))

  // marked emits bare `<h2>…</h2>`; the ids are added here so the table of
  // contents and any in-document anchors resolve.
  const html = marked
    .parser(tokens)
    .replace(
      /<h([2-4])>([\s\S]*?)<\/h\1>/g,
      (_match, level, inner) => `<h${level} id="${escapeHtml(slug(inner))}">${inner}</h${level}>`,
    )

  return { title, html, toc }
}
