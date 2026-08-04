import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { kindDef, kindIndex, summary } from '../test/fixtures'
import { useKeybindings } from '../store/keybindings'
import { useUI } from '../store/ui'
import { CommandPalette } from './CommandPalette'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue([]) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

/*
 * The palette's command rows used to carry hand-written hints — a literal
 * "⌘Z" that said Command on a machine with no Command key and went on saying
 * it after the user rebound undo. They come from the registry now, and these
 * are the two ways that can regress.
 */

const kinds = kindIndex([kindDef('character')])
const nodes = [summary({ id: 'kael', name: 'Kael Vantris' })]

function open() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={qc}>
      <CommandPalette
        nodes={nodes}
        kinds={kinds}
        onJump={vi.fn()}
        onNewNode={vi.fn()}
        readOnly={false}
      />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  useKeybindings.setState({ overrides: {} })
  useUI.setState({ paletteOpen: true, shortcutsOpen: false })
})

describe('the command rows', () => {
  it('print the binding that is currently in force', () => {
    open()
    // jsdom is not a Mac, so this is the Ctrl notation rather than the glyph.
    expect(screen.getByRole('button', { name: /Toggle navigator/ })).toHaveTextContent('[')
    expect(screen.getByRole('button', { name: /New entity…/ })).toHaveTextContent('Ctrl+N')
  })

  it('follow a rebinding rather than repeating what the build shipped with', () => {
    useKeybindings.getState().setBinding('node.new', 'Mod+Shift+N')
    open()
    expect(screen.getByRole('button', { name: /New entity…/ })).toHaveTextContent('Ctrl+Shift+N')
  })

  it('say nothing at all where the user has removed the key', () => {
    useKeybindings.getState().setBinding('panel.inspector', null)
    open()
    expect(screen.getByRole('button', { name: /Toggle inspector/ })).not.toHaveTextContent(']')
  })

  it('offer the whole map, for anyone who has never pressed one of these', () => {
    open()
    fireEvent.click(screen.getByRole('button', { name: /Keyboard shortcuts/ }))
    expect(useUI.getState()).toMatchObject({ paletteOpen: false, shortcutsOpen: true })
  })
})
