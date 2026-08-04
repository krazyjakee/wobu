import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chordParts } from '../lib/keys'
import { useKeybindings } from '../store/keybindings'
import { useContextMenu } from '../hooks/useContextMenu'
import { ContextMenu, MenuItem, MenuLabel, MenuSeparator } from './ContextMenu'

/**
 * The menu primitive, apart from any surface that uses it (#130).
 *
 * Everything here is a promise made to a keyboard: that the menu can be opened
 * without a mouse, walked without a mouse, refused with a reason that can be
 * read without a mouse, and left without losing your place. A context menu that
 * only answers the right button is an accelerator half the users of this app
 * are not offered.
 */

const chose = vi.fn()
const carded = vi.fn()

/**
 * A row that behaves like the surfaces this is used on: a container that is not
 * itself a control, positioned with `transform` the way every virtualized tile
 * in Wobu is, holding a text field and a button.
 */
function Card() {
  const menu = useContextMenu<string>()
  return (
    <div
      data-testid="card"
      style={{ transform: 'translate(40px, 60px)' }}
      tabIndex={-1}
      onClick={carded}
      {...menu.trigger('Kael')}
    >
      <input aria-label="Notes" />
      <button type="button">Inside</button>
      {menu.anchor && (
        <ContextMenu
          x={menu.anchor.x}
          y={menu.anchor.y}
          onClose={menu.close}
          restoreFocus={menu.anchor.opener}
          label={`Actions for ${menu.anchor.item}`}
        >
          <MenuLabel>Character</MenuLabel>
          <MenuItem onSelect={chose}>Open</MenuItem>
          <MenuSeparator />
          <MenuItem
            disabledReason="A world has one style guide, so there is nothing to copy it into."
            onSelect={chose}
          >
            Duplicate
          </MenuItem>
          <MenuItem command="nav.toggleAll" onSelect={chose}>
            Collapse everything
          </MenuItem>
        </ContextMenu>
      )}
    </div>
  )
}

function card() {
  return screen.getByTestId('card')
}

beforeEach(() => {
  chose.mockReset()
  carded.mockReset()
  useKeybindings.setState({ overrides: {} })
})

describe('opening a menu', () => {
  it('answers the right button, Shift+F10 and the Menu key alike', () => {
    render(<Card />)

    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })
    expect(screen.getByRole('menu', { name: 'Actions for Kael' })).toBeInTheDocument()
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Open' }), { key: 'Escape' })

    fireEvent.keyDown(card(), { key: 'F10', shiftKey: true })
    expect(screen.getByRole('menu')).toBeInTheDocument()
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Open' }), { key: 'Escape' })

    fireEvent.keyDown(card(), { key: 'ContextMenu' })
    expect(screen.getByRole('menu')).toBeInTheDocument()
  })

  it('puts focus on the first row and takes every row out of the tab order', () => {
    render(<Card />)
    fireEvent.keyDown(card(), { key: 'F10', shiftKey: true })

    const items = within(screen.getByRole('menu')).getAllByRole('menuitem')
    expect(items[0]).toHaveFocus()
    for (const item of items) expect(item).toHaveAttribute('tabindex', '-1')
  })

  /*
   * The route that matters on a tile. A card is a container of controls rather
   * than a control, so it is not a tab stop — the keystroke arrives at whatever
   * button inside it the user had reached, and has to rise from there. Focus
   * goes back to that same control, not to the card.
   */
  it('rises from the control the user was on, and hands focus back to it', () => {
    render(<Card />)
    const inside = screen.getByRole('button', { name: 'Inside' })
    inside.focus()

    fireEvent.keyDown(inside, { key: 'F10', shiftKey: true })
    expect(screen.getByRole('menu')).toBeInTheDocument()

    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Open' }), { key: 'Escape' })
    expect(inside).toHaveFocus()
  })

  it('leaves a text field the platform’s own menu', () => {
    render(<Card />)
    const notes = screen.getByLabelText('Notes')

    const opened = fireEvent.contextMenu(notes, { clientX: 10, clientY: 10 })
    // Not prevented, so the webview goes on to draw cut, copy and paste.
    expect(opened).toBe(true)
    fireEvent.keyDown(notes, { key: 'F10', shiftKey: true })

    expect(screen.queryByRole('menu')).toBeNull()
  })
})

describe('walking a menu', () => {
  it('cycles with the arrows, Home and End, keeping the refused row in the ring', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })
    const items = within(screen.getByRole('menu')).getAllByRole('menuitem')
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      'Open',
      'Duplicate',
      // The chord is part of the row, printed from the registry.
      expect.stringContaining('Collapse everything'),
    ])

    fireEvent.keyDown(items[0] as HTMLElement, { key: 'ArrowDown' })
    // A refused row is `aria-disabled`, not `disabled`, so it is landed on and
    // can be asked why rather than being skipped in silence (#129).
    expect(items[1]).toHaveFocus()
    expect(items[1]).toHaveAttribute('aria-disabled', 'true')

    fireEvent.keyDown(items[1] as HTMLElement, { key: 'End' })
    expect(items[2]).toHaveFocus()
    fireEvent.keyDown(items[2] as HTMLElement, { key: 'ArrowDown' })
    expect(items[0]).toHaveFocus()
    fireEvent.keyDown(items[0] as HTMLElement, { key: 'ArrowUp' })
    expect(items[2]).toHaveFocus()
    fireEvent.keyDown(items[2] as HTMLElement, { key: 'Home' })
    expect(items[0]).toHaveFocus()
  })

  it('says why a refused row is refused, and does nothing when it is chosen', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })
    const refused = screen.getByRole('menuitem', { name: 'Duplicate' })

    fireEvent.focus(refused)
    expect(screen.getByRole('tooltip')).toHaveTextContent(/nothing to copy it into/)

    fireEvent.click(refused)
    expect(chose).not.toHaveBeenCalled()
    expect(screen.getByRole('menu')).toBeInTheDocument()
  })

  it('does not open a second menu from inside the first', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Open' }), {
      key: 'F10',
      shiftKey: true,
    })

    expect(screen.getAllByRole('menu')).toHaveLength(1)
  })
})

describe('leaving a menu', () => {
  it('closes on Escape and puts focus back on the row that opened it', () => {
    render(<Card />)
    fireEvent.keyDown(card(), { key: 'ContextMenu' })

    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Open' }), { key: 'Escape' })

    expect(screen.queryByRole('menu')).toBeNull()
    expect(card()).toHaveFocus()
  })

  it('closes when a row is chosen, and runs it', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    fireEvent.click(screen.getByRole('menuitem', { name: 'Open' }))

    expect(chose).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole('menu')).toBeNull()
    expect(card()).toHaveFocus()
  })

  it('closes when the surface underneath it scrolls out from under it', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    fireEvent.scroll(card())

    expect(screen.queryByRole('menu')).toBeNull()
  })
})

describe('where a menu is drawn', () => {
  it('escapes the transformed card it was declared inside', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    // A `position: fixed` menu inside a `transform`ed ancestor is positioned
    // against that ancestor rather than the viewport, which is every tile in
    // every grid in this app. Portalling is what keeps the coordinates honest.
    expect(screen.getByRole('menu').parentElement).toBe(document.body)
  })

  it('keeps its clicks to itself, so the card underneath is not also activated', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    fireEvent.click(screen.getByRole('menuitem', { name: 'Open' }))

    // React propagates portalled events to the tree the portal was written in,
    // so without the menu stopping them, choosing a row would also open the
    // card the menu belongs to.
    expect(carded).not.toHaveBeenCalled()
  })
})

describe('a row that is an accelerator for a command', () => {
  it('prints the binding in force, and follows a rebinding', () => {
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    const row = screen.getByRole('menuitem', { name: /Collapse everything/ })
    for (const part of chordParts('Mod+Shift+C')) {
      expect(row).toHaveTextContent(part)
    }
    expect(row).toHaveAttribute('aria-keyshortcuts')

    fireEvent.keyDown(row, { key: 'Escape' })
    useKeybindings.setState({ overrides: { 'nav.toggleAll': 'Mod+Alt+E' } })
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    const rebound = screen.getByRole('menuitem', { name: /Collapse everything/ })
    for (const part of chordParts('Mod+Alt+E')) {
      expect(rebound).toHaveTextContent(part)
    }
    expect(rebound).not.toHaveTextContent('Shift')
  })

  it('prints nothing at all for a command the user unbound', () => {
    useKeybindings.setState({ overrides: { 'nav.toggleAll': null } })
    render(<Card />)
    fireEvent.contextMenu(card(), { clientX: 40, clientY: 70 })

    const row = screen.getByRole('menuitem', { name: /Collapse everything/ })
    expect(row.querySelector('kbd')).toBeNull()
    expect(row).not.toHaveAttribute('aria-keyshortcuts')
  })
})
