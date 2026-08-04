# wobu.app

The public website, published to the `gh-pages` branch by
[`.github/workflows/website.yml`](../.github/workflows/website.yml) and served at
<https://wobu.app>.

It is a static build: plain HTML and one stylesheet, no client-side JavaScript, no framework. The
only dependency is [`marked`](https://marked.js.org/), used to render the legal documents. It has
its own `package.json` deliberately — the site must never add a dependency to the application's.

## Commands

```sh
cd website
npm ci
npm run build      # writes ./dist
npm run preview    # serves ./dist on http://localhost:4173
npm start          # build, then preview
```

`build.mjs` also takes `--out DIR` and `--root DIR` (the repository root it reads inputs from).

## What it builds

| Output | Source |
| --- | --- |
| `index.html` | `pages/home.mjs` — claims sourced from `README.md`, `docs/01-vision.md`, `docs/09-roadmap.md` |
| `download.html` | `pages/download.mjs` — links to GitHub Releases, install notes from `docs/12-releasing.md` |
| `legal.html` | `pages/legal.mjs` |
| `legal/privacy-policy.html` | `docs/legal/privacy-policy.md` |
| `legal/terms.html` | `docs/legal/terms.md` |
| `legal/licence.html` | `LICENSE` |
| `guide/**` | rendered from the Markdown in `docs/guide/`, the same source the app reads |
| `CNAME`, `.nojekyll`, `robots.txt`, `styles.css`, `favicon.svg` | `static/` |
| `sitemap.xml` | generated from the page list |

The repository files are **inputs only**; nothing outside `website/` is written to. Every one of
them is optional: if a legal document, the guide, or `LICENSE` is missing the build still succeeds,
prints a warning, and publishes a stub page so no link on the site is dead.

## Conventions

- Download links point at `/releases/latest`. Never hard-code a version number here.
- Links between pages are page-relative, so the output works from `wobu.app`, from a project-path
  Pages URL, from `npm run preview`, and from `file://`.
- Design tokens in `static/styles.css` are copied from `src/styles/tokens.css`; keep them in step so
  the site, the guide and the app stay one visual identity.
- Only describe behaviour the roadmap says is implemented. `docs/09-roadmap.md` is the source of
  truth for feature status.
- The repository's Prettier config applies here: run `npx prettier --write website` from the
  repository root after editing.
