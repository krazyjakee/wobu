/**
 * Values that appear on more than one page, or that would otherwise be typed
 * slightly differently each time. Nothing here is a claim about the product;
 * the claims live in `pages/` and are sourced from `README.md` and `docs/`.
 */
export const site = {
  name: 'Wobu',
  url: 'https://wobu.app',
  domain: 'wobu.app',
  description:
    'Wobu is a local-first, AI-assisted desktop app for building coherent fictional worlds and ' +
    'producing consistent concept art.',
  repo: 'https://github.com/krazyjakee/wobu',
  // Deliberately version-less. A hard-coded number on a website is wrong the
  // day after a release; `/releases/latest` is right forever.
  releases: 'https://github.com/krazyjakee/wobu/releases',
  latestRelease: 'https://github.com/krazyjakee/wobu/releases/latest',
  issues: 'https://github.com/krazyjakee/wobu/issues',
  roadmap: 'https://github.com/krazyjakee/wobu/blob/main/docs/09-roadmap.md',
  releaseGuide:
    'https://github.com/krazyjakee/wobu/blob/main/docs/12-releasing.md#installing-an-unsigned-beta',
  notices: 'https://github.com/krazyjakee/wobu/blob/main/THIRD-PARTY-NOTICES.md',
  year: 2026,
  author: 'Jake Cattrall',
}

/** Prefix that turns a site-root-relative path into a page-relative one. */
export function relativePrefix(depth) {
  return depth === 0 ? '' : '../'.repeat(depth)
}

export function escapeHtml(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}
