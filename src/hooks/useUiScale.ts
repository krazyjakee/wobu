import { useEffect } from 'react'
import { useSettings } from '../store/settings'

/**
 * Applies the interface scale to the document root.
 *
 * `zoom` rather than a root `font-size`: the token set in `styles/tokens.css`
 * is px throughout — deliberately, since it was ported verbatim from the
 * prototype — and the fixed rail and inspector widths are px too. Changing the
 * root font size would therefore move nothing at all. `zoom` scales the whole
 * layout, which is what someone reaching for this actually wants; it is the
 * difference between "bigger text overflowing the same boxes" and "a bigger
 * interface".
 */
export function useUiScale() {
  const scale = useSettings((s) => s.uiScale)
  useEffect(() => {
    const root = document.documentElement
    // Left unset at 1 rather than written as "1", so the default case creates
    // no stacking or containing-block effects at all.
    if (scale === 1) root.style.removeProperty('zoom')
    else root.style.zoom = String(scale)
  }, [scale])
}
