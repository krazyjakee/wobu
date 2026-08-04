import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { COMMANDS, useKeybindings } from '../store/keybindings'
import { useUI } from '../store/ui'
import { ShortcutsSheet } from './ShortcutsSheet'

/*
 * The answer to "which keys does this thing listen for", which before this
 * sheet existed could only be had by reading `useKeyboard.ts`. So the
 * assertions are about completeness and about honesty when a binding has been
 * changed — a reference that is merely decorative is worse than none, because
 * it is believed.
 */

beforeEach(() => {
  useKeybindings.setState({ overrides: {} })
  useUI.setState({ shortcutsOpen: true, mode: 'library' })
})

/** The row whose term begins with this label, as `<dt>` and `<dd>`. */
function row(label: string): HTMLElement {
  const term = screen.getByText(label, { selector: 'dt' })
  return term.parentElement as HTMLElement
}

describe('the keyboard reference', () => {
  it('stays shut until asked for', () => {
    useUI.setState({ shortcutsOpen: false })
    render(<ShortcutsSheet />)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('lists every command in the registry, and nothing it cannot explain', () => {
    render(<ShortcutsSheet />)
    for (const def of COMMANDS) {
      expect(screen.getByText(def.label, { selector: 'dt' })).toBeInTheDocument()
    }
  })

  it('prints the chord in this platform’s notation', () => {
    render(<ShortcutsSheet />)
    // jsdom is not a Mac, so the modifier reads as Ctrl rather than ⌘.
    expect(within(row('Command palette')).getByText('Ctrl')).toBeInTheDocument()
    expect(within(row('Command palette')).getByText('K')).toBeInTheDocument()
    expect(within(row('Toggle the navigator')).getByText('[')).toBeInTheDocument()
  })

  it('shows what a rebound command now answers to, not what it shipped with', () => {
    useKeybindings.getState().setBinding('mode.assets', 'Mod+J')
    render(<ShortcutsSheet />)
    expect(within(row('Assets')).getByText('J')).toBeInTheDocument()
  })

  it('says so when a command has been left with no key at all', () => {
    useKeybindings.getState().setBinding('panel.inspector', null)
    render(<ShortcutsSheet />)
    expect(within(row('Toggle the inspector')).getByText('no shortcut')).toBeInTheDocument()
  })

  it('names the command that has taken a contested chord', () => {
    // The failure being prevented: a user rebinds something, the key appears to
    // do nothing, and the list still cheerfully prints it as working.
    useKeybindings.getState().setBinding('mode.assets', 'Mod+K')
    render(<ShortcutsSheet />)

    const assets = row('Assets')
    expect(assets).toHaveTextContent(/runs Command palette instead/)
    expect(within(assets).getByText('no shortcut')).toBeInTheDocument()
  })

  it('includes the fixed keys it does not own, so the list is the whole truth', () => {
    render(<ShortcutsSheet />)
    expect(row('Close a dialog')).toHaveTextContent(/Escape dismisses whatever is on top/)
  })

  it('sends the reader to Settings to change any of it', () => {
    render(<ShortcutsSheet />)
    fireEvent.click(screen.getByRole('button', { name: 'Change these' }))
    expect(useUI.getState()).toMatchObject({ shortcutsOpen: false, mode: 'settings' })
  })
})
