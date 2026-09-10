import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { useKeybindings } from '../store/keybindings'
import { useUI } from '../store/ui'
import { ModeRail } from './ModeRail'

beforeEach(() => {
  useUI.setState({ mode: 'library' })
  useKeybindings.setState({ overrides: {} })
})

describe('ModeRail accessibility', () => {
  it('names the navigation and exposes exactly which destination is current', () => {
    render(<ModeRail />)

    const rail = screen.getByRole('navigation', { name: 'Workspace modes' })
    expect(screen.getByRole('button', { name: 'Library' })).toHaveAttribute('aria-current', 'page')
    expect(screen.getByRole('button', { name: 'Forge' })).not.toHaveAttribute('aria-current')

    fireEvent.click(screen.getByRole('button', { name: 'Forge' }))

    expect(screen.getByRole('button', { name: 'Forge' })).toHaveAttribute('aria-current', 'page')
    expect(screen.getByRole('button', { name: 'Library' })).not.toHaveAttribute('aria-current')
    expect(rail.querySelectorAll('[aria-current="page"]')).toHaveLength(1)
  })

  it('carries the shortcut for each mode, and follows it when it is rebound', () => {
    // The rail was the only visible route to a mode while the keys that also
    // reach them said so nowhere. It reads the registry rather than a literal,
    // so a rebound key does not leave a tooltip lying about it.
    const { rerender } = render(<ModeRail />)
    const forge = () => screen.getByRole('button', { name: 'Forge' })
    fireEvent.focusIn(forge())
    expect(screen.getByRole('tooltip')).toHaveTextContent('Forge · Ctrl+\\')
    expect(forge()).toHaveAttribute('aria-keyshortcuts', 'Control+\\')

    useKeybindings.getState().setBinding('mode.forge', 'Mod+G')
    rerender(<ModeRail />)

    expect(screen.getByRole('tooltip')).toHaveTextContent('Forge · Ctrl+G')
  })

  it('reaches the Narrative workspace without disturbing the other three', () => {
    render(<ModeRail />)
    const order = [...screen.getByRole('navigation', { name: 'Workspace modes' }).children]
      .map((child) => child.getAttribute('aria-label'))
      .filter((label): label is string => label !== null)
    // Appended, not inserted: a hand that reaches for the third button still
    // lands on Assets.
    expect(order.slice(0, 4)).toEqual(['Library', 'Forge', 'Assets', 'Narrative'])

    fireEvent.click(screen.getByRole('button', { name: 'Narrative' }))

    expect(useUI.getState().mode).toBe('narrative')
    expect(screen.getByRole('button', { name: 'Narrative' })).toHaveAttribute(
      'aria-current',
      'page',
    )
  })

  it('keeps the mode name out of the tooltip alone', () => {
    // The tooltip is a *description*. If it were the label, a rail with no
    // pointer on it would be four unnamed buttons.
    render(<ModeRail />)
    expect(screen.queryByRole('tooltip')).toBeNull()
    expect(screen.getByRole('button', { name: 'Assets' })).toBeInTheDocument()
  })
})
