import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { Combobox, type ComboboxOption } from './Combobox'
import { Modal } from './Modal'

const PEOPLE: ComboboxOption[] = [
  { value: 'kael', label: 'Kael' },
  { value: 'mira', label: 'Mira' },
  { value: 'ashen', label: 'The Ashen Gate' },
  { value: 'elan', label: 'Élan' },
]

function Harness({
  options = PEOPLE,
  initial = '',
  onPick,
  ...rest
}: {
  options?: ComboboxOption[]
  initial?: string
  onPick?: (value: string) => void
  sort?: 'title' | 'none'
  disabled?: boolean
}) {
  const [value, setValue] = useState(initial)
  return (
    <Combobox
      label="Subject"
      value={value}
      options={options}
      onChange={(next) => {
        setValue(next)
        onPick?.(next)
      }}
      {...rest}
    />
  )
}

const box = () => screen.getByRole('combobox', { name: 'Subject' })
const optionNames = () => screen.getAllByRole('option').map((row) => row.textContent)
const activeName = () => {
  const id = box().getAttribute('aria-activedescendant')
  return id ? document.getElementById(id)?.textContent : null
}

describe('combobox roles and state', () => {
  it('reports itself closed, with no list, until it is opened', () => {
    render(<Harness initial="kael" />)
    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(box()).toHaveValue('Kael')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('owns the listbox it opens and points at the highlighted row', () => {
    render(<Harness initial="mira" />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })

    const list = screen.getByRole('listbox')
    expect(box()).toHaveAttribute('aria-expanded', 'true')
    expect(box()).toHaveAttribute('aria-controls', list.id)
    // Opening lands on the current value rather than the top of the list.
    expect(activeName()).toBe('Mira')
    expect(within(list).getByRole('option', { name: 'Mira' })).toHaveAttribute(
      'aria-selected',
      'true',
    )
    expect(within(list).getByRole('option', { name: 'Kael' })).toHaveAttribute(
      'aria-selected',
      'false',
    )
  })

  it('numbers every row against the size of the filtered list', () => {
    render(<Harness />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    const rows = screen.getAllByRole('option')
    expect(rows.map((row) => row.getAttribute('aria-posinset'))).toEqual(['1', '2', '3', '4'])
    expect(new Set(rows.map((row) => row.getAttribute('aria-setsize')))).toEqual(new Set(['4']))
  })
})

describe('combobox keyboard operation', () => {
  it('opens on either arrow without choosing anything', () => {
    const onPick = vi.fn()
    render(<Harness onPick={onPick} />)
    fireEvent.keyDown(box(), { key: 'ArrowUp' })
    expect(box()).toHaveAttribute('aria-expanded', 'true')
    expect(onPick).not.toHaveBeenCalled()
  })

  it('walks the list with the arrows and stops at both ends', () => {
    render(<Harness />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(activeName()).toBe('Kael')

    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(activeName()).toBe('Mira')
    fireEvent.keyDown(box(), { key: 'ArrowUp' })
    expect(activeName()).toBe('Kael')
    // No wrap: pressing past the top holds, it does not jump to the bottom.
    fireEvent.keyDown(box(), { key: 'ArrowUp' })
    expect(activeName()).toBe('Kael')
  })

  it('jumps to the ends with Home and End', () => {
    render(<Harness />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'End' })
    expect(activeName()).toBe('Élan')
    fireEvent.keyDown(box(), { key: 'Home' })
    expect(activeName()).toBe('Kael')
  })

  it('commits the highlighted row on Enter and closes', () => {
    const onPick = vi.fn()
    render(<Harness onPick={onPick} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'Enter' })

    expect(onPick).toHaveBeenCalledWith('mira')
    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(box()).toHaveValue('Mira')
    expect(box()).toHaveFocus()
  })

  it('leaves Enter alone while closed, so the surrounding form still submits', () => {
    const submit = vi.fn()
    render(
      <form onSubmit={submit}>
        <Harness />
      </form>,
    )
    const event = fireEvent.keyDown(box(), { key: 'Enter' })
    // `fireEvent` returns false when a handler called preventDefault.
    expect(event).toBe(true)
  })

  it('abandons the search on Escape, keeping the value and the focus', () => {
    const onPick = vi.fn()
    render(<Harness initial="kael" onPick={onPick} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.change(box(), { target: { value: 'mir' } })
    fireEvent.keyDown(box(), { key: 'Escape' })

    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(box()).toHaveValue('Kael')
    expect(box()).toHaveFocus()
    expect(onPick).not.toHaveBeenCalled()
  })

  it('closes on Alt+ArrowUp and opens without moving on Alt+ArrowDown', () => {
    render(<Harness initial="mira" />)
    fireEvent.keyDown(box(), { key: 'ArrowDown', altKey: true })
    expect(activeName()).toBe('Mira')
    fireEvent.keyDown(box(), { key: 'ArrowDown', altKey: true })
    expect(activeName()).toBe('Mira')
    fireEvent.keyDown(box(), { key: 'ArrowUp', altKey: true })
    expect(box()).toHaveAttribute('aria-expanded', 'false')
  })

  it('gives up the list on Tab without changing the value', () => {
    const onPick = vi.fn()
    render(<Harness initial="kael" onPick={onPick} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'Tab' })

    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(box()).toHaveValue('Kael')
    expect(onPick).not.toHaveBeenCalled()
  })
})

describe('combobox filtering', () => {
  it('filters as the user types and highlights the best match', () => {
    render(<Harness />)
    fireEvent.change(box(), { target: { value: 'ash' } })

    expect(optionNames()).toEqual(['The Ashen Gate'])
    expect(activeName()).toBe('The Ashen Gate')
  })

  it('ignores accents in both directions', () => {
    render(<Harness />)
    fireEvent.change(box(), { target: { value: 'elan' } })
    expect(optionNames()).toEqual(['Élan'])
  })

  it('says so when nothing matches, and refuses to commit', () => {
    const onPick = vi.fn()
    render(<Harness onPick={onPick} />)
    fireEvent.change(box(), { target: { value: 'zzz' } })

    expect(screen.queryAllByRole('option')).toHaveLength(0)
    expect(screen.getByText('No match.')).toBeInTheDocument()
    fireEvent.keyDown(box(), { key: 'Enter' })
    expect(onPick).not.toHaveBeenCalled()
  })

  it('announces how many rows are left', async () => {
    const view = render(<Harness />)
    const live = () => view.container.querySelector('[aria-live="polite"]')
    // Present before it has anything to say, so the region is registered.
    expect(live()).toBeInTheDocument()

    fireEvent.change(box(), { target: { value: 'a' } })
    await waitFor(() => expect(live()).toHaveTextContent('4 results'))
    fireEvent.change(box(), { target: { value: 'ash' } })
    await waitFor(() => expect(live()).toHaveTextContent('1 result'))
  })

  it('drops the query when the list closes, so the box states the value again', () => {
    render(<Harness initial="kael" />)
    fireEvent.change(box(), { target: { value: 'mir' } })
    fireEvent.keyDown(box(), { key: 'Enter' })
    expect(box()).toHaveValue('Mira')

    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(box()).toHaveValue('')
    // The value the user is about to replace stays legible behind the search.
    expect(box()).toHaveAttribute('placeholder', 'Mira')
  })
})

describe('combobox ordering', () => {
  it('keeps the caller order by default', () => {
    render(<Harness />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(optionNames()).toEqual(['Kael', 'Mira', 'The Ashen Gate', 'Élan'])
  })

  it('files titles under their first real word when asked to sort', () => {
    render(<Harness sort="title" />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(optionNames()).toEqual(['The Ashen Gate', 'Élan', 'Kael', 'Mira'])
  })
})

describe('combobox grouping', () => {
  const grouped: ComboboxOption[] = [
    { value: 'a', label: 'Ashgate', group: 'Setting' },
    { value: 'b', label: 'Kael', group: 'Character' },
    { value: 'c', label: 'Vashk', group: 'Character' },
    { value: 'd', label: 'Harbour', group: 'Setting' },
  ]

  it('brings each group together under one heading, in first-appearance order', () => {
    render(<Harness options={grouped} sort="title" />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })

    const list = screen.getByRole('listbox')
    expect([...list.querySelectorAll('.cbx-group')].map((h) => h.textContent)).toEqual([
      'Setting',
      'Character',
    ])
    expect(optionNames()).toEqual(['Ashgate', 'Harbour', 'Kael', 'Vashk'])
  })

  it('keeps one heading per group even when the filter re-ranks the rows', () => {
    render(<Harness options={grouped} sort="title" />)
    fireEvent.change(box(), { target: { value: 'a' } })

    const list = screen.getByRole('listbox')
    expect([...list.querySelectorAll('.cbx-group')].map((h) => h.textContent)).toEqual([
      'Setting',
      'Character',
    ])
  })

  it('points each row at the heading above it, so the group is announced', () => {
    render(<Harness options={grouped} sort="title" />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })

    const row = screen.getByRole('option', { name: /Kael/ })
    const heading = document.getElementById(row.getAttribute('aria-describedby') ?? '')
    expect(heading).toHaveTextContent('Character')
  })
})

describe('combobox disabled rows', () => {
  const withDisabled: ComboboxOption[] = [
    { value: 'a', label: 'Cover', disabled: true },
    { value: 'b', label: 'Palette' },
    { value: 'c', label: 'Pose', disabled: true },
    { value: 'd', label: 'Silhouette' },
  ]

  it('offers a taken row, announces it, and steps over it', () => {
    const onPick = vi.fn()
    render(<Harness options={withDisabled} onPick={onPick} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })

    expect(screen.getByRole('option', { name: 'Cover' })).toHaveAttribute('aria-disabled', 'true')
    // Opening on a taken row would make Enter do nothing; it settles on the
    // first row that can actually be chosen.
    expect(activeName()).toBe('Palette')
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(activeName()).toBe('Silhouette')

    fireEvent.click(screen.getByRole('option', { name: 'Cover' }))
    expect(onPick).not.toHaveBeenCalled()
  })
})

describe('combobox in a sheet', () => {
  it('takes Escape for its own list before the dialog sees it', () => {
    const onClose = vi.fn()
    render(
      <Modal titleId="t" descriptionId="d" onClose={onClose}>
        <h2 id="t">Sheet</h2>
        <p id="d">A sheet with a picker in it.</p>
        <Harness initial="kael" />
      </Modal>,
    )

    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'Escape' })
    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(onClose).not.toHaveBeenCalled()

    // Closed, Escape belongs to the sheet again.
    fireEvent.keyDown(box(), { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('draws its list inside the sheet host, which the dialog does not mark inert', () => {
    render(
      <Modal titleId="t" descriptionId="d" onClose={() => {}}>
        <h2 id="t">Sheet</h2>
        <p id="d">A sheet with a picker in it.</p>
        <Harness />
      </Modal>,
    )
    fireEvent.keyDown(box(), { key: 'ArrowDown' })

    const list = screen.getByRole('listbox')
    expect(list.closest('[data-modal-host]')).not.toBeNull()
    expect(list.closest('[inert]')).toBeNull()
  })
})

describe('combobox with a very long list', () => {
  const many: ComboboxOption[] = Array.from({ length: 3000 }, (_, i) => ({
    value: `n${i}`,
    label: `Node ${i}`,
  }))

  it('draws a window rather than three thousand rows', () => {
    render(<Harness options={many} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    const drawn = screen.getAllByRole('option')
    expect(drawn.length).toBeGreaterThan(0)
    expect(drawn.length).toBeLessThan(60)
    // Windowed or not, the list still reports its true length to a reader.
    expect(drawn[0]).toHaveAttribute('aria-setsize', '3000')
  })

  it('keeps the row the keyboard is on in the document however far down it is', () => {
    render(<Harness options={many} />)
    fireEvent.keyDown(box(), { key: 'ArrowDown' })
    fireEvent.keyDown(box(), { key: 'End' })

    expect(activeName()).toBe('Node 2999')
    expect(screen.getByRole('option', { name: 'Node 2999' })).toBeInTheDocument()
  })

  it('narrows to a handful once the user types', () => {
    render(<Harness options={many} />)
    fireEvent.change(box(), { target: { value: 'node 2999' } })
    expect(optionNames()).toEqual(['Node 2999'])
  })
})

describe('combobox mouse operation', () => {
  it('opens on a press in the box and closes on a second one', () => {
    render(<Harness />)
    fireEvent.mouseDown(box())
    expect(box()).toHaveAttribute('aria-expanded', 'true')
    fireEvent.mouseDown(box())
    expect(box()).toHaveAttribute('aria-expanded', 'false')
  })

  it('chooses the row that was clicked and returns focus to the box', () => {
    const onPick = vi.fn()
    render(<Harness onPick={onPick} />)
    fireEvent.mouseDown(box())
    fireEvent.click(screen.getByRole('option', { name: 'The Ashen Gate' }))

    expect(onPick).toHaveBeenCalledWith('ashen')
    expect(box()).toHaveFocus()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('dismisses on a press outside without changing the value', () => {
    const onPick = vi.fn()
    render(
      <div>
        <Harness initial="kael" onPick={onPick} />
        <button>Elsewhere</button>
      </div>,
    )
    fireEvent.mouseDown(box())
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Elsewhere' }))

    expect(box()).toHaveAttribute('aria-expanded', 'false')
    expect(onPick).not.toHaveBeenCalled()
  })
})
