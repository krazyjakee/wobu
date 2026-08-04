import { useState } from 'react'
import { useUI, type Banner } from '../store/ui'
import { PRESENCE_BANNER } from '../lib/presence'
import { Icon } from './Icon'
import { IconButton } from './Tooltip'

/**
 * The glyph for a code, falling back to the padlock.
 *
 * Presence is listed here rather than left to the fallback because a padlock is
 * the one thing it must not say: nothing about a collaborator having a node open
 * locks anything.
 */
const ICON: Record<string, string> = {
  'share.unmounted': 'share',
  [PRESENCE_BANNER]: 'share',
}

/**
 * The persistent half of the error surface.
 *
 * A banner stays until the condition behind it is gone, so it says what is
 * wrong with the *workspace* rather than what went wrong with one action —
 * the share is not there, the folder cannot be written to. Toasts handle
 * everything else; see `errorSurface` in `src/lib/api.ts` for the split.
 *
 * Dismissal is deliberately allowed even though the condition persists: a
 * user who has read it and is dealing with it should not have to read it
 * again for the rest of the session, and the next failing command raises it
 * back anyway.
 */
export function Banners() {
  const banners = useUI((s) => s.banners)
  const clear = useUI((s) => s.clearBanner)

  // Rendered even when empty, and deliberately not `return null`. `.app` is a
  // four-row grid and its children are placed in order, so a component that
  // sometimes renders nothing would shift the workspace and the status bar up
  // a row whenever the share was fine — which is almost always. An empty flex
  // container has no height, so this costs a node and not a pixel.
  return (
    <div className="banners" role="status" aria-live="polite">
      {banners.map((b) => (
        <BannerRow key={b.code} banner={b} onDismiss={() => clear(b.code)} />
      ))}
    </div>
  )
}

function BannerRow({ banner, onDismiss }: { banner: Banner; onDismiss: () => void }) {
  const [open, setOpen] = useState(false)

  return (
    <div className="banner" data-code={banner.code}>
      <Icon name={ICON[banner.code] ?? 'lock'} size="sm" />
      <div className="banner-body">
        <span className="banner-text">{banner.text}</span>
        {/* The detail is the OS's own wording. Useful in a bug report, noise
            in a banner, so it starts folded. */}
        {banner.detail && (
          <>
            <button className="banner-more" onClick={() => setOpen((v) => !v)}>
              {open ? 'Hide details' : 'Details'}
            </button>
            {open && <code className="banner-detail">{banner.detail}</code>}
          </>
        )}
      </div>
      {banner.action && (
        <button className="banner-act" onClick={banner.action.run}>
          {banner.action.label}
        </button>
      )}
      {!banner.sticky && (
        <IconButton
          className="banner-x"
          label="Dismiss"
          tip="Hide this banner. The condition it reports is unchanged and it comes back if it happens again."
          placement="left"
          onClick={onDismiss}
        >
          <Icon name="x" size="sm" />
        </IconButton>
      )}
    </div>
  )
}
