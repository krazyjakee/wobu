import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { useUI } from '../store/ui'
import { ModeRail } from './ModeRail'

beforeEach(() => useUI.setState({ mode: 'library' }))

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
})
