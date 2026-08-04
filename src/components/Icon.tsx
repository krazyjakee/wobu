import type { CSSProperties } from 'react'

export type IconSize = 'sm' | 'md' | 'xl'

/**
 * The unit square every glyph in `<IconSprite>` is drawn on.
 *
 * This was the bug behind half of #128. The sprite is authored on a 24×24 grid,
 * but the `<svg>` that consumes it carried no `viewBox` at all — so there was no
 * mapping from those 24 units to the 16px box `.ic` gives it, one user unit was
 * one CSS pixel, and every glyph was rendered at 150% and then *cropped* to its
 * top-left corner. "Icons are too large for their containers and most are
 * visually cut off" is exactly what a missing `viewBox` looks like.
 */
const GRID = 24

/** Nominal box in px, matching `.ic`, `.ic-sm` and `.ic-xl` in `base.css`. */
const BOX: Record<IconSize, number> = { sm: 14, md: 16, xl: 38 }

const CLS: Record<IconSize, string> = { sm: 'ic ic-sm', md: 'ic', xl: 'ic ic-xl' }

/**
 * The stroke every icon draws, in rendered pixels, at every size.
 *
 * A `stroke-width` in the stylesheet is in *user units*, so one number across
 * three box sizes is three different weights on screen: a 38px `ic-xl` drew its
 * outline two and a half times heavier than a 16px `ic` beside it. The weight
 * is therefore set here, per size, in the only unit a reader can see — and
 * inline, so it beats the class rules rather than arguing with them.
 */
const STROKE_PX = 1.3

export function Icon({
  name,
  size = 'md',
  style,
  className,
}: {
  /** sprite id without the `i-` prefix, e.g. `species` */
  name: string
  size?: IconSize
  style?: CSSProperties
  className?: string
}) {
  const strokeWidth = Math.round((GRID / BOX[size]) * STROKE_PX * 100) / 100
  return (
    <svg
      viewBox={`0 0 ${GRID} ${GRID}`}
      className={className ? `${CLS[size]} ${className}` : CLS[size]}
      style={{ strokeWidth, ...style }}
      aria-hidden
    >
      <use href={`#i-${name}`} />
    </svg>
  )
}
