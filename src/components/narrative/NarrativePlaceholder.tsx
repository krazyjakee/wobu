import type { ReactNode } from 'react'
import { Icon } from '../Icon'

/**
 * A pane that has a place in the workspace but nothing to put in it yet.
 *
 * The point of this component is that it is *not* a spinner and not an empty
 * box. A reader who opens Flow in this build should learn, without leaving the
 * pane, that the canvas is genuinely absent rather than slow, broken, or
 * waiting on a selection they have not made. It carries the lock glyph the rest
 * of Wobu uses for "you cannot do this here", plus the sentence saying why.
 */
export function NarrativePlaceholder({
  title,
  reason,
  children,
}: {
  title: string
  reason: string
  /** Affordances that belong with the explanation, usually refused ones. */
  children?: ReactNode
}) {
  return (
    <div className="nrt-placeholder empty-state">
      <Icon name="lock" size="xl" />
      <h3>{title}</h3>
      <p>{reason}</p>
      {children && <div className="nrt-placeholder-actions">{children}</div>}
    </div>
  )
}
