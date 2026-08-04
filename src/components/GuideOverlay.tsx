import { useEffect, useMemo, useRef, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { report, useUI } from '../store/ui'
import {
  GUIDE_GROUPS,
  GUIDE_PAGES,
  guidePage,
  searchGuide,
  useGuide,
  type GuidePage,
} from './GuideContent'
import { GuideMarkdown, type GuideLinkHandlers } from './GuideMarkdown'
import { Modal } from './Modal'

/**
 * The guide, in the window.
 *
 * It is a modal rather than a mode because reading it is something you do
 * *while* stuck on something else: the world you were looking at is still
 * behind it, and Escape puts you back exactly where you were. Nothing here
 * touches the network — the pages are in the bundle — so the guide is readable
 * on a plane, on a dead share, and before a single key has been pasted.
 */
export function GuideOverlay() {
  const slug = useGuide((s) => s.slug)
  if (slug === null) return null
  return <OpenGuide slug={slug} />
}

function OpenGuide({ slug }: { slug: string }) {
  const anchor = useGuide((s) => s.anchor)
  const open = useGuide((s) => s.open)
  const close = useGuide((s) => s.close)
  const setShortcutsOpen = useUI((s) => s.setShortcutsOpen)
  const setMode = useUI((s) => s.setMode)
  const [query, setQuery] = useState('')
  const body = useRef<HTMLDivElement>(null)

  const page: GuidePage = guidePage(slug) ?? GUIDE_PAGES[0]!
  const hits = useMemo(() => searchGuide(query), [query])

  useEffect(() => {
    const container = body.current
    if (!container) return
    const target = anchor ? container.querySelector(`#${CSS.escape(anchor)}`) : null
    if (target) target.scrollIntoView?.({ block: 'start' })
    else container.scrollTop = 0
  }, [slug, anchor])

  const handlers: GuideLinkHandlers = {
    onNavigate: (target, hash) => open(target, hash),
    onExternal: (href) => {
      void openUrl(href).catch((e: unknown) => report(e, 'Could not open the browser'))
    },
    onAction: (action) => {
      if (action === 'shortcuts') {
        close()
        setShortcutsOpen(true)
        return
      }
      if (action === 'settings') {
        close()
        setMode('settings')
      }
    },
  }

  const index = GUIDE_PAGES.indexOf(page)
  const previous = index > 0 ? GUIDE_PAGES[index - 1] : undefined
  const next = index >= 0 ? GUIDE_PAGES[index + 1] : undefined

  return (
    <Modal
      className="sheet guide"
      titleId="guide-title"
      descriptionId="guide-description"
      onClose={close}
    >
      <div className="guide-head">
        <h2 id="guide-title">Guide</h2>
        <p id="guide-description" className="guide-sub">
          {GUIDE_PAGES.length} pages, all of them in this window and none of them online.
        </p>
        <input
          className="guide-search"
          type="search"
          value={query}
          placeholder="Search the guide"
          aria-label="Search the guide"
          data-modal-initial-focus
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>

      <div className="guide-body">
        <nav className="guide-nav" aria-label="Guide contents">
          {query.trim().length >= 2 ? (
            <div className="guide-results">
              <p className="guide-nav-group">
                {hits.length === 0
                  ? 'No matches'
                  : `${hits.length} ${hits.length === 1 ? 'page' : 'pages'}`}
              </p>
              {hits.map((hit) => (
                <button
                  type="button"
                  key={`${hit.page.slug}-${hit.section?.id ?? ''}`}
                  className="guide-result"
                  onClick={() => {
                    open(hit.page.slug, hit.section?.id ?? null)
                    setQuery('')
                  }}
                >
                  <span className="guide-result-title">
                    {hit.page.title}
                    {hit.section && (
                      <span className="guide-result-section">{hit.section.text}</span>
                    )}
                  </span>
                  <span className="guide-result-excerpt">{hit.excerpt}</span>
                </button>
              ))}
            </div>
          ) : (
            GUIDE_GROUPS.map((group) => (
              <div key={group.title}>
                <p className="guide-nav-group">{group.title}</p>
                {group.pages.map((entry) => (
                  <button
                    type="button"
                    key={entry.slug}
                    className="guide-nav-link"
                    aria-current={entry.slug === page.slug ? 'page' : undefined}
                    onClick={() => open(entry.slug)}
                  >
                    {entry.title}
                  </button>
                ))}
              </div>
            ))
          )}
        </nav>

        <div className="guide-page" ref={body} tabIndex={-1}>
          <article className="guide-prose">
            <GuideMarkdown markdown={page.markdown} handlers={handlers} />
          </article>

          <div className="guide-pagenav">
            {previous ? (
              <button type="button" className="guide-step" onClick={() => open(previous.slug)}>
                <span>Previous</span>
                {previous.title}
              </button>
            ) : (
              <span />
            )}
            {next && (
              <button type="button" className="guide-step next" onClick={() => open(next.slug)}>
                <span>Next</span>
                {next.title}
              </button>
            )}
          </div>
        </div>
      </div>
    </Modal>
  )
}
