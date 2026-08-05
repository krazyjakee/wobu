import { escapeHtml, relativePrefix, site } from './site.mjs'

/**
 * Every link on the site is page-relative rather than root-relative, so the
 * built output works unchanged from `wobu.app`, from a project-path GitHub
 * Pages URL, from `npx serve`, and from `file://`.
 */
const NAV = [
  { id: 'home', href: 'index.html', label: 'Overview' },
  { id: 'guide', href: 'guide/index.html', label: 'Guide' },
  { id: 'download', href: 'download.html', label: 'Download' },
  { id: 'legal', href: 'legal.html', label: 'Legal' },
]

const FOOTER = [
  {
    heading: 'Product',
    links: [
      { href: 'index.html', label: 'Overview' },
      { href: 'guide/index.html', label: 'User guide' },
      { href: 'download.html', label: 'Download' },
      { href: site.roadmap, label: 'Roadmap', external: true },
    ],
  },
  {
    heading: 'Legal',
    links: [
      { href: 'legal.html', label: 'Legal overview' },
      { href: 'legal/privacy-policy.html', label: 'Privacy policy' },
      { href: 'legal/terms.html', label: 'Terms of use' },
      { href: 'legal/licence.html', label: 'MIT licence' },
    ],
  },
  {
    heading: 'Source',
    links: [
      { href: site.repo, label: 'GitHub repository', external: true },
      { href: site.releases, label: 'Releases', external: true },
      { href: site.issues, label: 'Issue tracker', external: true },
      { href: site.notices, label: 'Third-party notices', external: true },
    ],
  },
]

function href(target, rel) {
  return /^(https?:|mailto:|#)/.test(target) ? target : rel + target
}

function link({ href: target, label, external }, rel) {
  const attrs = external ? ' rel="noopener"' : ''
  return `<a href="${href(target, rel)}"${attrs}>${escapeHtml(label)}</a>`
}

/**
 * The app's mark: an authored node, its descendant, and that descendant's
 * descendant, each hanging off the elbow the navigator tree draws. Kept in step
 * with `branding/wobu-mark.svg`, which the build substitutes when the checkout
 * has it — this copy is the fallback so the site never renders unbranded.
 */
const DEFAULT_MARK = `<svg class="mark" viewBox="66 68 372 372" aria-hidden="true" focusable="false">
        <g fill="none" stroke-width="30" stroke-linecap="round" stroke-linejoin="round">
          <path d="M140 176 V 226 Q140 258 172 258 H 216" stroke="#e2a44f" />
          <path d="M256 296 V 350 Q256 382 288 382 H 332" stroke="#4fd1c5" />
        </g>
        <circle cx="140" cy="136" r="58" fill="#e2a44f" />
        <circle cx="256" cy="256" r="54" fill="#4fd1c5" />
        <circle cx="372" cy="380" r="50" fill="#9d7cf5" />
      </svg>`

let markSvg = DEFAULT_MARK

/** Replaces the built-in mark with the repository's master artwork. */
export function useMark(svg) {
  markSvg = svg
}

function mark() {
  return markSvg
}

function header(current, rel) {
  const items = NAV.map((item) => {
    const active = item.id === current ? ' aria-current="page"' : ''
    return `<a href="${rel + item.href}"${active}>${escapeHtml(item.label)}</a>`
  }).join('\n          ')

  return `<header class="site-header">
      <div class="wrap header-inner">
        <a class="brand" href="${rel}index.html">
          ${mark()}
          <span class="wordmark">wobu</span>
        </a>
        <nav class="site-nav" aria-label="Primary">
          ${items}
          <a class="nav-source" href="${site.repo}" rel="noopener">GitHub</a>
        </nav>
      </div>
    </header>`
}

function footer(rel) {
  const columns = FOOTER.map(
    (column) => `<div class="footer-column">
            <h2>${escapeHtml(column.heading)}</h2>
            <ul>
              ${column.links.map((item) => `<li>${link(item, rel)}</li>`).join('\n              ')}
            </ul>
          </div>`,
  ).join('\n          ')

  return `<footer class="site-footer">
      <div class="wrap">
        <div class="footer-grid">
          <div class="footer-about">
            <p class="footer-brand">${mark()}<span class="wordmark">wobu</span></p>
            <p>
              Wobu runs no servers. There is no account, no tracking and no update check. The AI
              work happens on your own machine, or on your own account with a service you chose.
            </p>
          </div>
          ${columns}
        </div>
        <p class="footer-legal">
          © ${site.year} ${escapeHtml(site.author)}. Wobu is free software under the
          <a href="${rel}legal/licence.html">MIT licence</a>. This site is built from the
          <a href="${site.repo}" rel="noopener">Wobu source code</a>.
        </p>
      </div>
    </footer>`
}

/**
 * @param {object} options
 * @param {string} options.title      Text for `<title>`, without the site suffix.
 * @param {string} options.description Meta description and Open Graph description.
 * @param {string} options.main       Inner HTML of `<main>`.
 * @param {string} [options.nav]      Id of the nav entry to mark as current.
 * @param {number} [options.depth]    Directory depth of the output file.
 * @param {string} [options.canonical] Absolute canonical URL.
 * @param {string} [options.bodyClass]
 */
export function renderPage({
  title,
  description,
  main,
  nav = '',
  depth = 0,
  canonical,
  bodyClass = '',
}) {
  const rel = relativePrefix(depth)
  const fullTitle =
    title === site.name ? `${site.name} — ${site.tagline}` : `${title} — ${site.name}`
  const bodyAttr = bodyClass ? ` class="${bodyClass}"` : ''

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(fullTitle)}</title>
    <meta name="description" content="${escapeHtml(description)}" />
    <meta name="color-scheme" content="dark" />
    <meta name="theme-color" content="#0d0e12" />
    ${canonical ? `<link rel="canonical" href="${canonical}" />` : ''}
    <meta property="og:type" content="website" />
    <meta property="og:site_name" content="${site.name}" />
    <meta property="og:title" content="${escapeHtml(fullTitle)}" />
    <meta property="og:description" content="${escapeHtml(description)}" />
    ${canonical ? `<meta property="og:url" content="${canonical}" />` : ''}
    <meta name="twitter:card" content="summary" />
    <link rel="icon" href="${rel}favicon.svg" type="image/svg+xml" />
    <link rel="stylesheet" href="${rel}styles.css" />
  </head>
  <body${bodyAttr}>
    <a class="skip-link" href="#main">Skip to content</a>
    ${header(nav, rel)}
    <main id="main">
${main}
    </main>
    ${footer(rel)}
  </body>
</html>
`
}
