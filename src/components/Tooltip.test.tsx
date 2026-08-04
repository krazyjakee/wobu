import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { IconButton, TipButton, Tooltip, Truncated } from './Tooltip'

/**
 * These are accessibility tests, not render tests.
 *
 * Every assertion here is one of the ways `title` fails: it is invisible to the
 * keyboard, it cannot be dismissed, it is a name for some screen readers and a
 * description for others, and on a `disabled` control it never appears at all —
 * which is the case #129 was actually filed about.
 */

afterEach(() => {
  vi.useRealTimers()
})

describe('a tooltip and the keyboard', () => {
  it('opens on focus and wires itself as a description, not a name', () => {
    render(<IconButton label="Settings" />)
    const button = screen.getByRole('button', { name: 'Settings' })

    // Named before anything has hovered it. The tooltip is never the only
    // carrier of the label.
    expect(screen.queryByRole('tooltip')).toBeNull()

    fireEvent.focusIn(button)

    const tip = screen.getByRole('tooltip')
    expect(tip).toHaveTextContent('Settings')
    expect(button).toHaveAttribute('aria-describedby', tip.id)
    // Still named by its label, not by the description.
    expect(screen.getByRole('button', { name: 'Settings' })).toBe(button)
  })

  it('closes on blur and takes its description with it', () => {
    render(<IconButton label="Close" />)
    const button = screen.getByRole('button')

    fireEvent.focusIn(button)
    expect(screen.getByRole('tooltip')).toBeInTheDocument()

    fireEvent.focusOut(button)
    expect(screen.queryByRole('tooltip')).toBeNull()
    expect(button).not.toHaveAttribute('aria-describedby')
  })

  it('does not open because a click moved focus', () => {
    // Otherwise every button the user presses grows a label they were already
    // looking at, and it outlives the pointer that caused it.
    render(<IconButton label="Minimise" />)
    const button = screen.getByRole('button')

    fireEvent.pointerDown(button)
    fireEvent.focusIn(button)

    expect(screen.queryByRole('tooltip')).toBeNull()
  })

  it('dismisses on Escape without swallowing the key', () => {
    // A tooltip open inside a dialog must not become the reason Escape stopped
    // closing the dialog.
    const onEscape = vi.fn()
    document.addEventListener('keydown', onEscape)
    render(<IconButton label="Restore" />)
    const button = screen.getByRole('button')

    fireEvent.focusIn(button)
    expect(screen.getByRole('tooltip')).toBeInTheDocument()

    fireEvent.keyDown(button, { key: 'Escape' })

    expect(screen.queryByRole('tooltip')).toBeNull()
    expect(onEscape).toHaveBeenCalledTimes(1)
    document.removeEventListener('keydown', onEscape)
  })

  it('neither takes focus nor offers a tab stop of its own', () => {
    render(<IconButton label="Share" />)
    const button = screen.getByRole('button')
    button.focus()
    fireEvent.focusIn(button)

    const tip = screen.getByRole('tooltip')
    expect(document.activeElement).toBe(button)
    expect(tip).not.toHaveAttribute('tabindex')
    expect(tip.querySelectorAll('button, a, input, [tabindex]')).toHaveLength(0)
  })
})

describe('a tooltip and the pointer', () => {
  it('waits before appearing, and leaves when the pointer does', () => {
    vi.useFakeTimers()
    render(<IconButton label="Assets" />)
    const button = screen.getByRole('button')

    fireEvent.pointerEnter(button)
    expect(screen.queryByRole('tooltip')).toBeNull()

    act(() => vi.advanceTimersByTime(400))
    expect(screen.getByRole('tooltip')).toBeInTheDocument()

    fireEvent.pointerLeave(button)
    expect(screen.queryByRole('tooltip')).toBeNull()
  })

  it('does not linger over a control the pointer only crossed', () => {
    vi.useFakeTimers()
    render(<IconButton label="Library" />)
    const button = screen.getByRole('button')

    fireEvent.pointerEnter(button)
    act(() => vi.advanceTimersByTime(100))
    fireEvent.pointerLeave(button)
    act(() => vi.advanceTimersByTime(500))

    expect(screen.queryByRole('tooltip')).toBeNull()
  })
})

describe('a tooltip on the control it was given', () => {
  it('keeps the handlers that were already there', () => {
    const onFocus = vi.fn()
    const onClick = vi.fn()
    render(
      <Tooltip tip="Explains itself">
        <button onFocus={onFocus} onClick={onClick}>
          Do it
        </button>
      </Tooltip>,
    )
    const button = screen.getByRole('button')

    fireEvent.focusIn(button)
    fireEvent.click(button)

    expect(onFocus).toHaveBeenCalledTimes(1)
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('keeps an aria-describedby the control already had', () => {
    render(
      <>
        <span id="existing">the other description</span>
        <Tooltip tip="And this one">
          <button aria-describedby="existing">Do it</button>
        </Tooltip>
      </>,
    )
    const button = screen.getByRole('button')
    fireEvent.focusIn(button)

    expect(button.getAttribute('aria-describedby')).toBe(
      `existing ${screen.getByRole('tooltip').id}`,
    )
  })

  it('adds nothing to the DOM when there is nothing to say', () => {
    const { container } = render(
      <Tooltip tip={null}>
        <button>Do it</button>
      </Tooltip>,
    )
    // No wrapper element: a tooltip must never change how a flex row lays out.
    expect(container.firstElementChild?.tagName).toBe('BUTTON')
    expect(container.childElementCount).toBe(1)
  })
})

/*
 * The disabled case.
 *
 * A `disabled` button fires no pointer events, takes no focus, and — in this
 * codebase — also carries `pointer-events: none` from `.btn[disabled]`. So the
 * moment a user most wants an explanation is the exact moment the platform
 * guarantees they cannot get one. `TipButton` refuses with `aria-disabled`
 * instead and intercepts the activation itself.
 */
describe('a control that is refused rather than removed', () => {
  const refuse = (onClick = vi.fn()) => {
    render(
      <TipButton
        className="btn"
        disabledReason="Open a project first — there is nothing to write into."
        onClick={onClick}
      >
        New entity
      </TipButton>,
    )
    return { button: screen.getByRole('button', { name: 'New entity' }), onClick }
  }

  it('says so to assistive technology without leaving the tab order', () => {
    const { button } = refuse()
    expect(button).toHaveAttribute('aria-disabled', 'true')
    expect(button).not.toBeDisabled()
    button.focus()
    expect(document.activeElement).toBe(button)
  })

  it('can be asked why, by the keyboard', () => {
    const { button } = refuse()
    fireEvent.focusIn(button)
    expect(screen.getByRole('tooltip')).toHaveTextContent(
      'Open a project first — there is nothing to write into.',
    )
    expect(button).toHaveAttribute('aria-describedby', screen.getByRole('tooltip').id)
  })

  it('can be asked why, by the pointer, which a disabled button cannot', () => {
    vi.useFakeTimers()
    const { button } = refuse()
    fireEvent.pointerEnter(button)
    act(() => vi.advanceTimersByTime(400))
    expect(screen.getByRole('tooltip')).toHaveTextContent('Open a project first')
  })

  it('still refuses: the click never reaches the handler', () => {
    const { button, onClick } = refuse()
    fireEvent.click(button)
    expect(onClick).not.toHaveBeenCalled()
  })

  it('runs normally once the precondition is met', () => {
    const onClick = vi.fn()
    render(
      <TipButton disabledReason={null} tip="Write a new file" onClick={onClick}>
        New entity
      </TipButton>,
    )
    const button = screen.getByRole('button')
    expect(button).not.toHaveAttribute('aria-disabled')
    fireEvent.click(button)
    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('shows the reason in place of the ordinary tooltip, not beside it', () => {
    render(
      <TipButton tip="Write a new file" disabledReason="This project is read-only.">
        New entity
      </TipButton>,
    )
    fireEvent.focusIn(screen.getByRole('button'))
    const tip = screen.getByRole('tooltip')
    expect(tip).toHaveTextContent('This project is read-only.')
    expect(tip).not.toHaveTextContent('Write a new file')
  })
})

describe('text that is cut off', () => {
  /** jsdom lays nothing out, so the measurement it would take is staged here. */
  function withMeasurements(scrollWidth: number, clientWidth: number, run: () => void) {
    const scroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollWidth')
    const client = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')
    Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
      configurable: true,
      get: () => scrollWidth,
    })
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
      configurable: true,
      get: () => clientWidth,
    })
    try {
      run()
    } finally {
      if (scroll) Object.defineProperty(HTMLElement.prototype, 'scrollWidth', scroll)
      if (client) Object.defineProperty(HTMLElement.prototype, 'clientWidth', client)
    }
  }

  it('says nothing when the whole string is on screen', () => {
    withMeasurements(180, 180, () => {
      render(<Truncated as="code" text="/home/ana/worlds/reef" />)
      const text = screen.getByText('/home/ana/worlds/reef')
      fireEvent.focusIn(text)
      expect(screen.queryByRole('tooltip')).toBeNull()
      // And no tab stop, because it is holding nothing back.
      expect(text).not.toHaveAttribute('tabindex')
    })
  })

  it('becomes readable and reachable exactly when it is clipped', () => {
    withMeasurements(600, 180, () => {
      render(<Truncated as="code" text="/home/ana/worlds/the-drowned-reef-of-saint-elmo" />)
      const text = screen.getByText(/drowned-reef/)
      expect(text).toHaveAttribute('tabindex', '0')

      fireEvent.focusIn(text)
      expect(screen.getByRole('tooltip')).toHaveTextContent(
        '/home/ana/worlds/the-drowned-reef-of-saint-elmo',
      )
    })
  })
})
