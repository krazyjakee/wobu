import {
  cloneElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ReactElement,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import { useTruncated } from '../hooks/useTruncated'
import { placeTip, type TipPlacement, type TipPosition } from '../lib/tooltip'

/**
 * The one tooltip in Wobu (#129).
 *
 * `title` is not a tooltip. It appears after a delay the user cannot change,
 * cannot be styled, never appears on a touch screen, is announced by some
 * screen readers and not others, and vanishes the moment the pointer moves. It
 * is a fallback the platform kept, not a control surface — so none of the
 * tooltips here use it.
 *
 * Three rules this primitive exists to keep:
 *
 * 1. **A tooltip is never the only carrier of a name.** `<IconButton>` writes
 *    the same string into `aria-label`, so the control is named whether or not
 *    anything is hovering it. The tooltip is wired with `aria-describedby`,
 *    which is *description*, not name.
 * 2. **Keyboard focus opens it.** A keyboard user gets the same explanation a
 *    mouse user does, and `Escape` puts it away. It never takes focus itself —
 *    it is a portalled `<div role="tooltip">` with no tab stop and no focus
 *    trap, so tabbing past a control with a tooltip behaves as if it had none.
 * 3. **Disabled controls still explain themselves.** See `<TipButton>`.
 *
 * The trigger is cloned rather than wrapped, so adding a tooltip to a control
 * adds no box to the layout and cannot change how a flex row lays out.
 */

/** The handlers and attributes `<Tooltip>` merges into whatever it is given. */
interface TriggerProps {
  'aria-describedby'?: string
  onPointerEnter?: (event: { currentTarget: HTMLElement }) => void
  onPointerLeave?: () => void
  onPointerDown?: () => void
  onFocus?: (event: { currentTarget: HTMLElement }) => void
  onBlur?: () => void
  onClick?: (event: unknown) => void
}

/** Hover has to be deliberate; focus does not, because focus already was. */
const HOVER_DELAY_MS = 250

/**
 * Whether the focus about to arrive was caused by a click.
 *
 * A tooltip that opens because the user pressed the button they were already
 * pointing at is noise, and it then outlives the pointer that caused it.
 * `:focus-visible` says the same thing, but only in a browser that implements
 * it for `matches()`; this is decided from events every runtime dispatches, so
 * it behaves identically under test and in the app.
 *
 * Module scope because there is one pointer: the question is not "was *this*
 * tooltip's control clicked", it is "did the last press in this document move
 * the focus that is arriving now".
 */
const press = {
  armed: false,
  arm() {
    press.armed = true
  },
  /** Reads and disarms, so one press suppresses exactly one focus. */
  take(): boolean {
    const was = press.armed
    press.armed = false
    return was
  },
}

function join(...parts: (string | undefined)[]): string | undefined {
  const kept = parts.filter((part): part is string => !!part)
  return kept.length > 0 ? kept.join(' ') : undefined
}

export function Tooltip({
  tip,
  placement = 'top',
  children,
}: {
  /** Nothing is rendered and nothing is wired when this is empty. */
  tip?: string | null
  placement?: TipPlacement
  children: ReactElement
}) {
  const id = useId()
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const [pending, setPending] = useState<HTMLElement | null>(null)
  const [position, setPosition] = useState<TipPosition | null>(null)
  const bubble = useRef<HTMLDivElement | null>(null)
  const open = anchor !== null && !!tip

  const hide = useCallback(() => {
    setPending(null)
    setAnchor(null)
    setPosition(null)
  }, [])

  // The hover delay, as a subscription rather than a stored timer id: the
  // pointer leaving unsets `pending`, and React tears the timeout down for us,
  // including on unmount.
  useEffect(() => {
    if (!pending) return
    const timer = window.setTimeout(() => setAnchor(pending), HOVER_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [pending])

  // Escape dismisses whatever tooltip is showing, without consuming the key:
  // a tooltip open inside a dialog must not stop Escape closing the dialog.
  useEffect(() => {
    if (!open) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') hide()
    }
    document.addEventListener('keydown', onKeyDown, true)
    return () => document.removeEventListener('keydown', onKeyDown, true)
  }, [open, hide])

  // Measured after the bubble exists, then kept correct while the page moves
  // under it — a status bar tooltip whose control scrolled away is worse than
  // none, because it now labels something else.
  useLayoutEffect(() => {
    if (!anchor) return
    const update = () => {
      const node = bubble.current
      if (!node) return
      const box = anchor.getBoundingClientRect()
      setPosition(
        placeTip(
          { left: box.left, top: box.top, width: box.width, height: box.height },
          { width: node.offsetWidth, height: node.offsetHeight },
          placement,
          { width: window.innerWidth, height: window.innerHeight },
        ),
      )
    }
    update()
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [anchor, placement, tip])

  if (!tip) return children

  const given = children.props as TriggerProps
  const trigger = cloneElement(children as ReactElement<TriggerProps>, {
    'aria-describedby': join(given['aria-describedby'], open ? id : undefined),
    onPointerEnter: (event) => {
      given.onPointerEnter?.(event)
      setPending(event.currentTarget)
    },
    onPointerLeave: () => {
      given.onPointerLeave?.()
      hide()
    },
    onPointerDown: () => {
      given.onPointerDown?.()
      press.arm()
    },
    onFocus: (event) => {
      given.onFocus?.(event)
      if (!press.take()) setAnchor(event.currentTarget)
    },
    onBlur: () => {
      given.onBlur?.()
      press.take()
      hide()
    },
    onClick: (event) => {
      given.onClick?.(event)
      hide()
    },
  })

  return (
    <>
      {trigger}
      {open &&
        createPortal(
          <div
            ref={bubble}
            id={id}
            role="tooltip"
            className={`tip tip-${position?.placement ?? placement}`}
            style={{
              left: position?.left ?? 0,
              top: position?.top ?? 0,
              // Hidden for the one frame between mounting and measuring, so it
              // never flashes in the top-left corner on the way to its place.
              visibility: position ? 'visible' : 'hidden',
            }}
          >
            {tip}
          </div>,
          document.body,
        )}
    </>
  )
}

/**
 * A button that can say why it is unavailable.
 *
 * The `disabled` attribute is the reason #129 exists. A disabled control fires
 * no pointer events at all — `.btn[disabled]` in `base.css` even sets
 * `pointer-events: none` on top of that — so a tooltip attached to one can
 * never open, and it is removed from the tab order, so there is no keyboard
 * route to it either. The user is told "no" at exactly the moment the software
 * refuses to say why.
 *
 * So a `TipButton` with a `disabledReason` is not `disabled`. It is
 * `aria-disabled`, which means the same thing to assistive technology, but
 * leaves the button focusable, hoverable and therefore able to explain itself.
 * Activation is intercepted instead of prevented by the platform: the click is
 * swallowed here and `onClick` is never called, which is the same guarantee
 * `disabled` gives and the only one that mattered.
 */
export function TipButton({
  tip,
  disabledReason,
  placement,
  onClick,
  children,
  type = 'button',
  ...rest
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'disabled'> & {
  /** Shown when the button works. */
  tip?: string | null
  /** Set to refuse the button, and to say what would un-refuse it. */
  disabledReason?: string | null
  placement?: TipPlacement
}) {
  const refused = !!disabledReason
  return (
    <Tooltip tip={refused ? disabledReason : tip} placement={placement}>
      <button
        {...rest}
        type={type}
        aria-disabled={refused || undefined}
        onClick={(event) => {
          if (refused) {
            event.preventDefault()
            event.stopPropagation()
            return
          }
          onClick?.(event)
        }}
      >
        {children}
      </button>
    </Tooltip>
  )
}

/**
 * An icon-only button.
 *
 * `label` is required and is not optional-by-another-name: it becomes the
 * accessible name *and* the tooltip, so an icon-only control cannot be added
 * without one. `tip` is for the cases where the pointer should be told more
 * than the name — a keyboard chord, usually.
 */
export function IconButton({
  label,
  tip,
  ...rest
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'disabled' | 'aria-label'> & {
  label: string
  tip?: string | null
  disabledReason?: string | null
  placement?: TipPlacement
}) {
  return <TipButton {...rest} aria-label={label} tip={tip ?? label} />
}

/**
 * Text that explains itself only when it is cut off.
 *
 * Focusable exactly when it is truncated, which is the only moment it holds
 * something a keyboard user cannot otherwise read. A permanent tab stop on a
 * path that fits would be a cost with no payment.
 */
export function Truncated({
  text,
  as: Tag = 'span',
  className,
  placement,
}: {
  text: string
  as?: 'span' | 'code'
  className?: string
  placement?: TipPlacement
}) {
  const [ref, truncated] = useTruncated<HTMLElement>(text)
  const classes = [className, truncated ? 'tip-clip' : ''].filter(Boolean).join(' ')
  return (
    <Tooltip tip={truncated ? text : null} placement={placement}>
      <Tag ref={ref} className={classes || undefined} tabIndex={truncated ? 0 : undefined}>
        {text as ReactNode}
      </Tag>
    </Tooltip>
  )
}
