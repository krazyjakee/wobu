import { useState } from 'react'
import { useUI } from '../store/ui'
import { Icon } from './Icon'

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

  if (!banners.length) return null
  return (
    <div className="banners" role="status" aria-live="polite">
      {banners.map((b) => (
        <BannerRow
          key={b.code}
          code={b.code}
          text={b.text}
          detail={b.detail}
          onDismiss={() => clear(b.code)}
        />
      ))}
    </div>
  )
}

function BannerRow({
  code,
  text,
  detail,
  onDismiss,
}: {
  code: string
  text: string
  detail?: string
  onDismiss: () => void
}) {
  const [open, setOpen] = useState(false)

  return (
    <div className="banner" data-code={code}>
      <Icon name="lock" size="sm" />
      <div className="banner-body">
        <span className="banner-text">{text}</span>
        {/* The detail is the OS's own wording. Useful in a bug report, noise
            in a banner, so it starts folded. */}
        {detail && (
          <>
            <button className="banner-more" onClick={() => setOpen((v) => !v)}>
              {open ? 'Hide details' : 'Details'}
            </button>
            {open && <code className="banner-detail">{detail}</code>}
          </>
        )}
      </div>
      <button className="banner-x" onClick={onDismiss} aria-label="Dismiss">
        <Icon name="x" size="sm" />
      </button>
    </div>
  )
}
