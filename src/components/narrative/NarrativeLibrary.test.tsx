import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useUI } from '../../store/ui'
import { NarrativeLibrary } from './NarrativeLibrary'
import {
  NO_NARRATIVE_SOURCE,
  type NarrativeListState,
  type NarrativeSectionId,
} from './narrativeModel'

/**
 * The Library is where the five list states and the keyboard actually meet a
 * reader, so they are exercised through it rather than through the list on its
 * own: a state the section forgets to pass down is the same bug as a state the
 * list forgets to render.
 *
 * Rows are supplied by the test, never by the component. There is no narrative
 * backend in this build, and a component that could produce a scene by itself
 * would be inventing one.
 */
function sections(
  over: Partial<Record<NarrativeSectionId, NarrativeListState>> = {},
): Record<NarrativeSectionId, NarrativeListState> {
  return { ...NO_NARRATIVE_SOURCE, ...over }
}

const scenes: NarrativeListState = {
  kind: 'ready',
  items: [
    { id: 'council', name: 'Council hearing', status: 'needsText' },
    { id: 'verdict', name: 'The verdict', status: 'needsReview' },
    { id: 'aftermath', name: 'Aftermath', status: 'ready', conflict: 'Edited on two machines' },
  ],
}

beforeEach(() => {
  useUI.setState({
    bands: {},
    narrative: { sceneId: null, beatId: null, lineId: null },
    narrativeReveal: null,
    narrativeFilters: { needsText: false, needsReview: false, outOfDate: false },
  })
})

describe('what a section says when it has no rows', () => {
  it('admits it cannot answer, rather than showing an empty scene list', () => {
    // The distinction the whole workspace rests on: "this project has no
    // scenes" and "this build cannot read scenes" look identical if the
    // unavailable state is left to render as an empty list.
    render(<NarrativeLibrary sections={sections()} readOnly={false} />)
    expect(screen.getByRole('navigation', { name: 'Narrative library' })).toBeInTheDocument()
    expect(
      screen.getAllByText(/Facts, knowledge, relationships and quests/i).length,
    ).toBeGreaterThan(0)
    expect(screen.queryByRole('button', { name: 'Create first scene' })).toBeNull()
  })

  it('says it is reading, and marks the wait for a screen reader', () => {
    render(
      <NarrativeLibrary sections={sections({ scenes: { kind: 'loading' } })} readOnly={false} />,
    )
    expect(screen.getByText('Reading scenes…')).toHaveAttribute('aria-busy', 'true')
  })

  it('reports a failed read as an alert with the reason', () => {
    render(
      <NarrativeLibrary
        sections={sections({ scenes: { kind: 'error', message: 'permission denied' } })}
        readOnly={false}
      />,
    )
    expect(screen.getByRole('alert')).toHaveTextContent('Could not read scenes: permission denied')
  })

  it('creates the first scene for real, and still refuses the example', () => {
    // The half that works and the half that does not, side by side. Creating a
    // scene is a real write now; unpacking Ashfall needs a command that writes
    // a whole project at once, which does not exist — so it says so rather
    // than disappearing.
    const created = vi.fn()
    render(
      <NarrativeLibrary
        sections={sections({ scenes: { kind: 'ready', items: [] } })}
        readOnly={false}
        onCreateScene={created}
      />,
    )
    const create = screen.getByRole('button', { name: 'Create first scene' })
    expect(create).not.toHaveAttribute('aria-disabled')
    fireEvent.click(create)
    expect(created).toHaveBeenCalledOnce()

    const example = screen.getByRole('button', { name: 'Ashfall example' })
    expect(example).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focusIn(example)
    expect(screen.getByRole('tooltip')).toHaveTextContent('writes a whole project')
  })

  it('refuses to create in a read-only folder, with the reason a reader can act on', () => {
    render(
      <NarrativeLibrary
        sections={sections({ scenes: { kind: 'ready', items: [] } })}
        readOnly
        onCreateScene={undefined}
      />,
    )
    const create = screen.getByRole('button', { name: 'Create first scene' })
    expect(create).toHaveAttribute('aria-disabled', 'true')
    fireEvent.focusIn(create)
    expect(screen.getByRole('tooltip')).toHaveTextContent('read-only')
  })

  it('gives the read-only reason precedence over the missing-storage one', () => {
    // Both are true. The one a reader can do something about — open a writable
    // copy — is the one worth saying.
    render(
      <NarrativeLibrary sections={sections({ scenes: { kind: 'ready', items: [] } })} readOnly />,
    )
    fireEvent.focusIn(screen.getByRole('button', { name: 'Create first scene' }))
    expect(screen.getByRole('tooltip')).toHaveTextContent('read-only')
    expect(screen.getByText(/read-only, so nothing can be added/i)).toBeInTheDocument()
  })
})

describe('rows, and the one selection they write', () => {
  it('writes the shared selection when a scene is chosen', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    fireEvent.click(screen.getByText('Council hearing'))
    expect(useUI.getState().narrative).toEqual({
      sceneId: 'council',
      beatId: null,
      lineId: null,
    })
    expect(useUI.getState().narrativeReveal).toMatchObject({
      sceneId: 'council',
      origin: 'library',
    })
  })

  it('shows whatever the shared selection points at, whoever set it', () => {
    // Set as if the Flow canvas had done it. The Library never hears about that
    // directly; it reads the same field.
    useUI.getState().selectNarrative({ sceneId: 'verdict' }, 'flow')
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    expect(screen.getByRole('option', { name: /The verdict/ })).toHaveAttribute(
      'aria-selected',
      'true',
    )
  })

  it('says a status in words, not only in colour', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    expect(screen.getByRole('option', { name: /Council hearing/ })).toHaveTextContent('Needs text')
    expect(screen.getByRole('option', { name: /Aftermath/ })).toHaveTextContent('Conflict')
  })

  it('is a list rather than a listbox where a row cannot be selected yet', () => {
    render(
      <NarrativeLibrary
        sections={sections({
          quests: { kind: 'ready', items: [{ id: 'q1', name: 'The beacon' }] },
        })}
        readOnly={false}
      />,
    )
    expect(screen.getByRole('list', { name: 'Quests' })).toBeInTheDocument()
    expect(screen.queryByRole('listbox', { name: 'Quests' })).toBeNull()
  })
})

describe('a selection made somewhere else', () => {
  it('scrolls the row into view once per request, and not for its own clicks', () => {
    // jsdom has no `scrollIntoView`, which is also why the component calls it
    // optionally: a browser without it must not take the pane down.
    const scroll = vi.fn()
    Element.prototype.scrollIntoView = scroll
    const { rerender } = render(
      <NarrativeLibrary sections={sections({ scenes })} readOnly={false} />,
    )
    expect(scroll).not.toHaveBeenCalled()

    // As the Flow canvas would write it.
    useUI.getState().selectNarrative({ sceneId: 'aftermath' }, 'flow')
    rerender(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    expect(scroll).toHaveBeenCalledTimes(1)

    // A re-render with no new request must not scroll again, or the reader
    // could never look away from the selected row.
    rerender(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    expect(scroll).toHaveBeenCalledTimes(1)

    // The pane's own click is already in view.
    fireEvent.click(screen.getByText('Council hearing'))
    expect(scroll).toHaveBeenCalledTimes(1)
  })
})

describe('the keyboard', () => {
  it('moves down the rows with one tab stop for the whole list', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    const list = screen.getByRole('listbox', { name: 'Scenes' })
    const rows = within(list).getAllByRole('option')
    expect(rows.map((row) => row.getAttribute('tabindex'))).toEqual(['0', '-1', '-1'])

    rows[0]!.focus()
    fireEvent.keyDown(list, { key: 'ArrowDown' })
    expect(document.activeElement).toBe(rows[1])

    fireEvent.keyDown(list, { key: 'End' })
    expect(document.activeElement).toBe(rows[2])
    fireEvent.keyDown(list, { key: 'Home' })
    expect(document.activeElement).toBe(rows[0])
  })

  it('selects with Enter, because an option is not a button', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    const list = screen.getByRole('listbox', { name: 'Scenes' })
    fireEvent.keyDown(list, { key: 'ArrowDown' })
    fireEvent.keyDown(list, { key: 'Enter' })
    expect(useUI.getState().narrative.sceneId).toBe('verdict')
  })

  it('stops at the ends rather than wrapping past them', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    const list = screen.getByRole('listbox', { name: 'Scenes' })
    const rows = within(list).getAllByRole('option')
    fireEvent.keyDown(list, { key: 'ArrowUp' })
    expect(document.activeElement).toBe(rows[0])
  })
})

describe('the work filters', () => {
  it('narrows the rows to the states that were asked for', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    fireEvent.click(screen.getByRole('button', { name: /Needs text/ }))
    const rows = within(screen.getByRole('listbox', { name: 'Scenes' })).getAllByRole('option')
    expect(rows.map((row) => row.textContent)).toEqual(['Council hearingNeeds text'])
  })

  it('does not offer to create a first scene when a filter hid the ones there are', () => {
    // The bug this is here for: a filter empties the list, the empty state
    // fires, and a project with nine scenes is invited to write its first.
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    fireEvent.click(screen.getByRole('button', { name: /Out of date/ }))
    expect(screen.getByText(/is in the states you filtered to/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Create first scene' })).toBeNull()
  })

  it('cannot turn a failed read into an empty one', () => {
    render(
      <NarrativeLibrary
        sections={sections({ scenes: { kind: 'error', message: 'disk gone' } })}
        readOnly={false}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /Needs review/ }))
    expect(screen.getByRole('alert')).toHaveTextContent('disk gone')
  })
})

describe('the sections themselves', () => {
  it('opens by default and remembers being closed in the shared band state', () => {
    render(<NarrativeLibrary sections={sections({ scenes })} readOnly={false} />)
    const heading = screen.getByRole('button', { name: 'Scenes' })
    expect(heading).toHaveAttribute('aria-expanded', 'true')

    fireEvent.click(heading)

    expect(screen.getByRole('button', { name: 'Scenes' })).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('listbox', { name: 'Scenes' })).toBeNull()
    // The same record the Library navigator's headings use, so "collapse
    // everything" reaches these too.
    expect(useUI.getState().bands['narrative:scenes']).toBe(false)
  })
})
