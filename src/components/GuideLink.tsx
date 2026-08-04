import { openGuide } from './GuideContent'

/**
 * "Learn more", next to the thing it explains.
 *
 * A guide nobody can find is a guide nobody reads, and the moment a reader
 * wants the influence chapter is the moment they are looking at the influence
 * panel — not later, from a menu, having remembered the word for it. So every
 * major surface carries one of these, and it opens the guide at the page for
 * that surface rather than at the front.
 */
export function GuideLink({
  page,
  anchor = null,
  children = 'Learn more',
}: {
  page: string
  anchor?: string | null
  children?: string
}) {
  return (
    <button
      type="button"
      className="guide-link"
      title="Open the guide"
      onClick={() => openGuide(page, anchor)}
    >
      {children}
    </button>
  )
}

/** The rail's help affordance: the one place the guide is always reachable. */
export function GuideRailButton() {
  return (
    <button
      type="button"
      className="rbtn rail-guide"
      data-tip="Guide"
      aria-label="Guide"
      onClick={() => openGuide()}
    >
      ?
    </button>
  )
}
