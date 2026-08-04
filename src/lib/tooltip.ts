/**
 * The geometry and the measurement behind `<Tooltip>` (#129), kept out of the
 * component so both can be tested without a layout engine.
 *
 * jsdom reports every rectangle as zero, so anything that decided where a
 * tooltip goes *inside* the component would be untestable by construction —
 * which is how a tooltip ends up half off the screen on one monitor and nobody
 * notices for a release.
 */

export type TipPlacement = 'top' | 'bottom' | 'left' | 'right'

export interface TipRect {
  left: number
  top: number
  width: number
  height: number
}

export interface TipSize {
  width: number
  height: number
}

export interface TipPosition {
  left: number
  top: number
  /** Where it actually landed, which is not always where it was asked to go. */
  placement: TipPlacement
}

/** Distance from the control, in px. Enough to read as separate, not as a gap. */
export const TIP_GAP = 8

/** Distance kept from the window edge, so a flipped tooltip is never flush. */
const EDGE = 6

const OPPOSITE: Record<TipPlacement, TipPlacement> = {
  top: 'bottom',
  bottom: 'top',
  left: 'right',
  right: 'left',
}

function offer(
  anchor: TipRect,
  tip: TipSize,
  placement: TipPlacement,
): { left: number; top: number } {
  const centreX = anchor.left + anchor.width / 2 - tip.width / 2
  const centreY = anchor.top + anchor.height / 2 - tip.height / 2
  switch (placement) {
    case 'top':
      return { left: centreX, top: anchor.top - tip.height - TIP_GAP }
    case 'bottom':
      return { left: centreX, top: anchor.top + anchor.height + TIP_GAP }
    case 'left':
      return { left: anchor.left - tip.width - TIP_GAP, top: centreY }
    case 'right':
      return { left: anchor.left + anchor.width + TIP_GAP, top: centreY }
  }
}

/**
 * Whether a placement fits *on its own axis*.
 *
 * Only the axis matters: a tooltip that hangs off the left edge is slid back
 * along the bar it sits on, but one that hangs off the top has to move to the
 * other side of the control, because sliding it down would put it underneath.
 */
function fitsOnAxis(
  position: { left: number; top: number },
  tip: TipSize,
  placement: TipPlacement,
  viewport: TipSize,
): boolean {
  if (placement === 'top' || placement === 'bottom') {
    return position.top >= EDGE && position.top + tip.height <= viewport.height - EDGE
  }
  return position.left >= EDGE && position.left + tip.width <= viewport.width - EDGE
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), Math.max(low, high))
}

/**
 * Place a tooltip: try the asked-for side, flip to the opposite side if it does
 * not fit, then slide along the cross axis so it stays on screen either way.
 */
export function placeTip(
  anchor: TipRect,
  tip: TipSize,
  placement: TipPlacement,
  viewport: TipSize,
): TipPosition {
  let chosen = placement
  for (const candidate of [placement, OPPOSITE[placement]]) {
    if (fitsOnAxis(offer(anchor, tip, candidate), tip, candidate, viewport)) {
      chosen = candidate
      break
    }
  }
  const position = offer(anchor, tip, chosen)
  return {
    placement: chosen,
    left: clamp(position.left, EDGE, viewport.width - tip.width - EDGE),
    top: clamp(position.top, EDGE, viewport.height - tip.height - EDGE),
  }
}

/**
 * Whether an element is actually clipping its own content.
 *
 * The point of asking is that a tooltip repeating text the user can already
 * read is noise; one that reveals a path `text-overflow` swallowed is the only
 * way to see it. A pixel of slack because browsers round sub-pixel widths.
 */
export function isOverflowing(element: {
  scrollWidth: number
  clientWidth: number
  scrollHeight: number
  clientHeight: number
}): boolean {
  return (
    element.scrollWidth > element.clientWidth + 1 || element.scrollHeight > element.clientHeight + 1
  )
}
