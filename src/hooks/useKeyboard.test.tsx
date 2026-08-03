import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import type { NodeKind } from '../lib/api'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { Modal } from '../components/Modal'
import { useKeybindings } from '../store/keybindings'
import { useUI } from '../store/ui'
import { useActionShortcut, useKeyboard } from './useKeyboard'

const h = vi.hoisted(() => ({ undo: vi.fn(), redo: vi.fn() }))

// The dispatcher's only dependency on the world. Everything else it touches is
// the UI store, which is real here on purpose: these tests are about what a
// keystroke *does*, and asserting against a mocked store would prove nothing.
vi.mock('../lib/queries', () => ({
  useUndoRunner: () => ({ undo: h.undo, redo: h.redo }),
}))

const onNewNode = vi.fn()

function Harness({
  readOnly = false,
  navKinds = ['character', 'creature'],
  modal = false,
}: {
  readOnly?: boolean
  navKinds?: NodeKind[]
  modal?: boolean
}) {
  useKeyboard({ onNewNode, readOnly, navKinds })
  return (
    <>
      <div className="nav-search">
        <input placeholder="Filter world…" />
      </div>
      {modal && (
        <Modal titleId="t" descriptionId="d" onClose={() => {}}>
          <h2 id="t">A sheet</h2>
          <p id="d">Answer it.</p>
        </Modal>
      )}
    </>
  )
}

const reset = () =>
  useUI.setState({
    mode: 'library',
    tab: 'notes',
    navCollapsed: false,
    inspCollapsed: false,
    paletteOpen: false,
    shortcutsOpen: false,
    closedGroups: {},
    collapsedNodes: {},
    bands: {},
  })

beforeEach(() => {
  vi.clearAllMocks()
  useKeybindings.setState({ overrides: {} })
  reset()
})

describe('the default map', () => {
  it('opens and closes the palette, from anywhere including a text field', () => {
    render(<Harness />)
    const field = screen.getByPlaceholderText('Filter world…')

    // Explicitly allowed while typing: the palette is how you leave where you
    // are, and needing to click away first would defeat it.
    fireEvent.keyDown(field, { key: 'k', metaKey: true })
    expect(useUI.getState().paletteOpen).toBe(true)
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
    expect(useUI.getState().paletteOpen).toBe(false)
  })

  it('switches modes and editor tabs', () => {
    render(<Harness />)
    fireEvent.keyDown(window, { key: '\\', metaKey: true })
    expect(useUI.getState().mode).toBe('forge')
    fireEvent.keyDown(window, { key: '\\', metaKey: true })
    expect(useUI.getState().mode).toBe('library')

    fireEvent.keyDown(window, { key: ',', metaKey: true })
    expect(useUI.getState().mode).toBe('settings')

    // A tab shortcut takes you back to the Library, because that is the only
    // place the tab it selects is visible.
    fireEvent.keyDown(window, { key: '3', metaKey: true })
    expect(useUI.getState()).toMatchObject({ mode: 'library', tab: 'concepts' })
  })

  it('toggles the panes on the unmodified bracket keys, but never mid-word', () => {
    render(<Harness />)
    fireEvent.keyDown(window, { key: '[' })
    fireEvent.keyDown(window, { key: ']' })
    expect(useUI.getState()).toMatchObject({ navCollapsed: true, inspCollapsed: true })

    fireEvent.keyDown(screen.getByPlaceholderText('Filter world…'), { key: '[' })
    expect(useUI.getState().navCollapsed).toBe(true)
  })

  it('opens the shortcut reference, and closes it again on Escape', () => {
    render(<Harness />)
    fireEvent.keyDown(window, { key: '/', metaKey: true })
    expect(useUI.getState().shortcutsOpen).toBe(true)
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(useUI.getState().shortcutsOpen).toBe(false)
  })

  it('collapses everything, then opens it again', () => {
    render(<Harness />)
    fireEvent.keyDown(window, { key: 'C', metaKey: true, shiftKey: true })
    expect(useUI.getState().closedGroups).toEqual({ character: true, creature: true })

    fireEvent.keyDown(window, { key: 'C', metaKey: true, shiftKey: true })
    expect(useUI.getState().closedGroups).toEqual({})
  })

  it('reveals the navigator and puts the caret in its filter', async () => {
    render(<Harness />)
    useUI.setState({ navCollapsed: true, mode: 'assets' })

    fireEvent.keyDown(window, { key: 'f', metaKey: true })

    expect(useUI.getState()).toMatchObject({ mode: 'library', navCollapsed: false })
    await waitFor(() => expect(screen.getByPlaceholderText('Filter world…')).toHaveFocus())
  })
})

describe('undo and redo', () => {
  it('runs them, and yields the key to a text field', () => {
    render(<Harness />)
    fireEvent.keyDown(window, { key: 'z', metaKey: true })
    expect(h.undo).toHaveBeenCalledTimes(1)

    fireEvent.keyDown(window, { key: 'Z', metaKey: true, shiftKey: true })
    fireEvent.keyDown(window, { key: 'y', ctrlKey: true })
    expect(h.redo).toHaveBeenCalledTimes(2)

    // While the caret is in a textarea, ⌘Z belongs to the field.
    fireEvent.keyDown(screen.getByPlaceholderText('Filter world…'), { key: 'z', metaKey: true })
    expect(h.undo).toHaveBeenCalledTimes(1)
  })

  it('swallows the key on a read-only folder rather than failing at save time', () => {
    render(<Harness readOnly />)
    const dispatched = fireEvent.keyDown(window, { key: 'z', metaKey: true })
    expect(dispatched).toBe(false)
    expect(h.undo).not.toHaveBeenCalled()
  })
})

describe('while a dialog owns the screen', () => {
  it('leaves the workspace shortcuts alone', () => {
    // They would act on something behind the sheet the user is answering.
    render(<Harness modal />)
    fireEvent.keyDown(window, { key: '\\', metaKey: true })
    fireEvent.keyDown(window, { key: 'n', metaKey: true })
    expect(useUI.getState().mode).toBe('library')
    expect(onNewNode).not.toHaveBeenCalled()
  })

  it('does not stack the palette on top of it', () => {
    render(<Harness modal />)
    fireEvent.keyDown(window, { key: 'k', metaKey: true })
    expect(useUI.getState().paletteOpen).toBe(false)
  })
})

describe('after the user rebinds something', () => {
  it('follows the new chord and forgets the old one', () => {
    render(<Harness />)
    useKeybindings.getState().setBinding('mode.assets', 'Mod+J')

    fireEvent.keyDown(window, { key: 'j', metaKey: true })
    expect(useUI.getState().mode).toBe('assets')

    reset()
    fireEvent.keyDown(window, { key: 'A', metaKey: true, shiftKey: true })
    expect(useUI.getState().mode).toBe('library')
  })

  it('does nothing at all for a command they unbound', () => {
    render(<Harness />)
    useKeybindings.getState().setBinding('panel.navigator', null)
    const dispatched = fireEvent.keyDown(window, { key: '[' })
    expect(dispatched).toBe(true)
    expect(useUI.getState().navCollapsed).toBe(false)
  })
})

describe('a surface-owned action', () => {
  function Surface({ enabled = true }: { enabled?: boolean }) {
    useActionShortcut('enhance', enabled, h.undo)
    return <Harness />
  }

  it('runs on its own chord, and swallows it even when unavailable', () => {
    const { rerender } = render(<Surface />)
    expect(fireEvent.keyDown(window, { key: 'e', metaKey: true })).toBe(false)
    expect(h.undo).toHaveBeenCalledTimes(1)

    rerender(<Surface enabled={false} />)
    expect(fireEvent.keyDown(window, { key: 'e', ctrlKey: true })).toBe(false)
    expect(h.undo).toHaveBeenCalledTimes(1)
  })

  it('loses a chord it shares with an earlier command, rather than firing as well', () => {
    // The failure this prevents: two listeners, two different answers to what
    // one keystroke means, decided by whichever registered last.
    render(<Surface />)
    useKeybindings.getState().setBinding('enhance', 'Mod+K')

    fireEvent.keyDown(window, { key: 'k', metaKey: true })

    expect(h.undo).not.toHaveBeenCalled()
    expect(useUI.getState().paletteOpen).toBe(true)
  })
})
