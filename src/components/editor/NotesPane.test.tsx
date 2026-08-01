import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WobuNode } from '../../lib/api'
import { kindDef, node as buildNode } from '../../test/fixtures'
import { NotesPane } from './NotesPane'

const autosave = {
  queue: vi.fn<(patch: Partial<WobuNode>) => void>(),
  flush: vi.fn<() => void>(),
  status: 'idle' as const,
}

beforeEach(() => {
  autosave.queue.mockReset()
  autosave.flush.mockReset()
})

describe('notes receiving a remote edit', () => {
  it('does not replace local text under the active cursor and still flushes it', () => {
    const original = buildNode({ id: 'kael', notesRaw: 'before' })
    const view = render(
      <NotesPane node={original} def={undefined} readOnly={false} autosave={autosave} />,
    )
    const notes = screen.getByRole('textbox') as HTMLTextAreaElement

    fireEvent.focus(notes)
    fireEvent.change(notes, { target: { value: 'my unfinished paragraph' } })
    view.rerender(
      <NotesPane
        node={{ ...original, notesRaw: "Nadia's incoming paragraph" }}
        def={undefined}
        readOnly={false}
        autosave={autosave}
      />,
    )

    expect(notes.value).toBe('my unfinished paragraph')
    expect(autosave.queue).toHaveBeenLastCalledWith({ notesRaw: 'my unfinished paragraph' })

    fireEvent.blur(notes)
    expect(autosave.flush).toHaveBeenCalledOnce()
    expect(notes.value).toBe('my unfinished paragraph')
  })

  it('adopts incoming text while the field is idle', () => {
    const original = buildNode({ id: 'kael', notesRaw: 'before' })
    const view = render(
      <NotesPane node={original} def={undefined} readOnly={false} autosave={autosave} />,
    )

    view.rerender(
      <NotesPane
        node={{ ...original, notesRaw: "Nadia's incoming paragraph" }}
        def={undefined}
        readOnly={false}
        autosave={autosave}
      />,
    )

    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe(
      "Nadia's incoming paragraph",
    )
  })
})

describe('kind attributes', () => {
  it('places registry-generated controls beneath Raw notes', () => {
    const node = buildNode({ id: 'kael', attributes: { scale: 'human' } })
    render(
      <NotesPane
        node={node}
        def={kindDef('character', {
          attributes: [{ key: 'scale', label: 'Scale', valueKind: 'text' }],
        })}
        readOnly={false}
        autosave={autosave}
      />,
    )

    const rawNotesColumn = screen.getByRole('heading', { name: 'Raw notes' }).closest('.col')
    expect(rawNotesColumn).toContainElement(screen.getByLabelText('Scale'))
    expect(screen.getByText('Attributes')).toBeInTheDocument()
  })
})
