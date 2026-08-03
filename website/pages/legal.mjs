import { escapeHtml, site } from '../lib/site.mjs'

/*
 * The legal documents are owned by `docs/legal/` and only rendered here. If one
 * is missing at build time the build still succeeds and publishes a stub, so a
 * footer link is never dead and a missing document is never silently a 404.
 */

export function legalIndexPage(documents) {
  const items = documents
    .map(
      (doc) => `<li class="card">
            <h3><a href="legal/${doc.slug}.html">${escapeHtml(doc.label)}</a></h3>
            <p>${doc.summary}</p>
            ${doc.available ? '' : '<p class="pending">Not published yet.</p>'}
          </li>`,
    )
    .join('\n          ')

  const main = `      <section class="page-head">
        <div class="wrap">
          <p class="eyebrow">Legal</p>
          <h1>Licence, privacy and terms</h1>
          <p class="lede">
            Wobu is free software under the MIT licence, operates no servers and collects nothing.
            These are the documents that say so precisely. Each one also ships beside the binary in
            every installer and is shown in the app under Settings › Legal.
          </p>
        </div>
      </section>

      <section class="band" aria-labelledby="documents-heading">
        <div class="wrap">
          <h2 id="documents-heading">Documents</h2>
          <ul class="cards">
          ${items}
          </ul>
          <p class="measure">
            Attribution for the open-source components Wobu is built from is generated from the
            lockfiles into
            <a href="${site.notices}" rel="noopener"><code>THIRD-PARTY-NOTICES.md</code></a>, which
            ships beside the application and is shown in Settings › Licences.
          </p>
        </div>
      </section>`

  return {
    path: 'legal.html',
    nav: 'legal',
    depth: 0,
    title: 'Legal',
    description:
      "Wobu's MIT licence, privacy policy and terms of use. Wobu operates no servers, has no " +
      'account, and collects no telemetry.',
    main,
  }
}

function tableOfContents(toc) {
  if (toc.length < 3) return ''
  const items = toc
    .map((entry) => `<li><a href="#${entry.id}">${escapeHtml(entry.text)}</a></li>`)
    .join('\n            ')

  return `<nav class="toc" aria-labelledby="toc-heading">
          <h2 id="toc-heading">On this page</h2>
          <ol>
            ${items}
          </ol>
        </nav>`
}

export function legalDocumentPage({ slug, label, title, description, html, toc, sourceUrl }) {
  const main = `      <div class="wrap doc-wrap">
        ${tableOfContents(toc)}
        <article class="doc">
${html}
          <hr />
          <p class="doc-source">
            This page is rendered from
            <a href="${sourceUrl}" rel="noopener">the source document in the repository</a>, which
            is the same file shipped inside every installer.
          </p>
        </article>
      </div>`

  return {
    path: `legal/${slug}.html`,
    nav: 'legal',
    depth: 1,
    title: title || label,
    description,
    main,
    bodyClass: 'doc-page',
  }
}

export function legalStubPage({ slug, label, description }) {
  const main = `      <section class="page-head">
        <div class="wrap">
          <p class="eyebrow">Legal</p>
          <h1>${escapeHtml(label)}</h1>
          <p class="lede">
            This document has not been published to the website yet. The current text lives in the
            <a href="${site.repo}/tree/main/docs/legal" rel="noopener">repository</a> and ships
            beside the application in every installer.
          </p>
          <p><a class="text-link" href="../legal.html">Back to legal →</a></p>
        </div>
      </section>`

  return {
    path: `legal/${slug}.html`,
    nav: 'legal',
    depth: 1,
    title: label,
    description,
    main,
  }
}

export function licencePage(text) {
  const main = `      <div class="wrap doc-wrap">
        <article class="doc">
          <h1>MIT Licence</h1>
          <p>
            Wobu is released under the MIT licence. The text below is the <code>LICENSE</code> file
            from the repository, which also ships beside the application in every installer.
          </p>
          <pre class="code licence"><code>${escapeHtml(text.trim())}</code></pre>
          <p>
            The licences of the open-source components Wobu is built from are listed in
            <a href="${site.notices}" rel="noopener"><code>THIRD-PARTY-NOTICES.md</code></a>.
          </p>
          <p><a class="text-link" href="../legal.html">Back to legal →</a></p>
        </article>
      </div>`

  return {
    path: 'legal/licence.html',
    nav: 'legal',
    depth: 1,
    title: 'MIT Licence',
    description: 'The MIT licence Wobu is released under.',
    main,
    bodyClass: 'doc-page',
  }
}
