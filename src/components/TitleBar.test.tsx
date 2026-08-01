import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ProjectSummary } from '../lib/api'
import { TitleBar } from './TitleBar'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }))
vi.mock('./WindowControls', () => ({ WindowControls: () => null }))

const project: ProjectSummary = {
  id: 'ashfall',
  name: 'Ashfall',
  path: '/worlds/Ashfall.wobu',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}

describe('TitleBar project menu accessibility', () => {
  it('uses the shared menu behavior and returns focus to its expanded trigger', () => {
    const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <TitleBar project={project} chain={[]} selected={null} kinds={new Map()} />
      </QueryClientProvider>,
    )
    const opener = screen.getByRole('button', { name: 'Ashfall' })
    expect(opener).toHaveAttribute('aria-haspopup', 'menu')
    expect(opener).toHaveAttribute('aria-expanded', 'false')

    fireEvent.click(opener)

    expect(opener).toHaveAttribute('aria-expanded', 'true')
    const menu = screen.getByRole('menu', { name: 'Project actions for Ashfall' })
    const item = screen.getByRole('menuitem', { name: 'Close project' })
    expect(menu).toContainElement(item)
    expect(item).toHaveFocus()

    fireEvent.keyDown(item, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(opener).toHaveFocus()
    expect(opener).toHaveAttribute('aria-expanded', 'false')
  })
})
