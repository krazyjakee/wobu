import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { EnhanceReady, SectionDef, WobuNode } from '../../lib/api'
import { node as buildNode } from '../../test/fixtures'
import { EnhanceReview } from './EnhanceReview'
import type { EnhanceSession } from './useEnhanceSession'

const definitions: SectionDef[] = [
  { key: 'silhouette', label: 'Silhouette', valueKind: 'text' },
  { key: 'palette', label: 'Palette', valueKind: 'list' },
  { key: 'signature', label: 'Signature details', valueKind: 'list' },
  { key: 'never', label: 'Never', valueKind: 'list' },
]

const ready: EnhanceReady = {
  jobId: 'job-1',
  nodeId: 'kael',
  questions: ['What is engraved on the signet?'],
  description: {
    sections: {
      silhouette: { type: 'text', value: 'New forward-canted silhouette' },
      palette: { type: 'list', value: ['#101820', '#c2703a'] },
      signature: { type: 'list', value: ['Unlit lantern'] },
      never: { type: 'list', value: ['Heroic stance'] },
    },
  },
}

const current = {
  sections: {
    silhouette: { type: 'text' as const, value: 'Current narrow silhouette' },
    palette: { type: 'list' as const, value: ['#2b2118'] },
    signature: { type: 'list' as const, value: ['Unlit lantern'] },
  },
}

const actions = {
  start: vi.fn(),
  stop: vi.fn(),
  accept: vi.fn(),
  forceAccept: vi.fn(),
  reject: vi.fn(),
}

function session(over: Partial<EnhanceSession> = {}): EnhanceSession {
  return {
    active: true,
    candidate: ready,
    complete: true,
    running: false,
    stopped: false,
    failure: null,
    starting: false,
    accepting: false,
    discarding: false,
    refusedNode: null,
    ...actions,
    ...over,
  }
}

beforeEach(() => {
  Object.values(actions).forEach((action) => action.mockReset())
})

describe('EnhanceReview', () => {
  it('shows questions and a current/new diff for every registry section', () => {
    render(<EnhanceReview current={current} definitions={definitions} session={session()} />)

    const questions = screen.getByRole('complementary', { name: 'Questions from Enhance' })
    expect(questions).toHaveTextContent('Enhance left these open rather than invent an answer')
    expect(questions).toHaveTextContent('What is engraved on the signet?')

    const silhouette = screen.getByRole('heading', { name: 'Silhouette' }).closest('section')
    expect(silhouette).toHaveTextContent('Current narrow silhouette')
    expect(silhouette).toHaveTextContent('New forward-canted silhouette')
    expect(silhouette).toHaveTextContent('changed')

    const signature = screen.getByRole('heading', { name: 'Signature details' }).closest('section')
    expect(signature).toHaveTextContent('unchanged')
    // The hex is the swatch's accessible name as well as its tooltip: a 14px
    // square is unreadable, and a `title` on it was reachable by neither a
    // screen reader nor a keyboard (#129).
    expect(screen.getByRole('img', { name: '#101820' })).toBeInTheDocument()
    expect(screen.getByRole('img', { name: '#c2703a' })).toBeInTheDocument()
  })

  it('accepts only explicitly selected new sections over the current description', () => {
    render(<EnhanceReview current={current} definitions={definitions} session={session()} />)

    const silhouette = screen.getByRole('heading', { name: 'Silhouette' }).closest('section')
    fireEvent.click(within(silhouette as HTMLElement).getByRole('button', { name: 'Use new' }))
    fireEvent.click(screen.getByRole('button', { name: 'Accept selected (1)' }))

    expect(actions.accept).toHaveBeenCalledWith({
      sections: {
        ...current.sections,
        silhouette: { type: 'text', value: 'New forward-canted silhouette' },
      },
    })
  })

  it('can accept the complete answer or reject it without saving', () => {
    render(<EnhanceReview current={current} definitions={definitions} session={session()} />)

    fireEvent.click(screen.getByRole('button', { name: 'Accept all' }))
    expect(actions.accept).toHaveBeenCalledWith(ready.description)

    fireEvent.click(screen.getByRole('button', { name: 'Reject' }))
    expect(actions.reject).toHaveBeenCalledOnce()
  })

  it('keeps a stopped partial response visibly local and offers no accept action', () => {
    const partial = {
      ...ready,
      description: {
        sections: {
          silhouette: { type: 'text' as const, value: 'Partial words that arrived' },
        },
      },
    }
    render(
      <EnhanceReview
        current={current}
        definitions={definitions}
        session={session({ candidate: partial, complete: false, stopped: true })}
      />,
    )

    expect(
      screen.getByText(/half-finished draft stays on this screen and is not saved/i),
    ).toBeInTheDocument()
    expect(screen.getByText('Partial words that arrived')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Accept all' })).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss local draft' }))
    expect(actions.reject).toHaveBeenCalledOnce()
    expect(actions.accept).not.toHaveBeenCalled()
  })

  it('shows the guarded disk version before a force accept', () => {
    const guarded: WobuNode = buildNode({
      id: 'kael',
      descriptionState: 'edited',
      description: {
        sections: {
          silhouette: { type: 'text', value: 'Hand edit saved while Enhance ran' },
        },
      },
    })
    render(
      <EnhanceReview
        current={current}
        definitions={definitions}
        session={session({ refusedNode: guarded })}
      />,
    )

    expect(screen.getByRole('alert')).toHaveTextContent('current description was hand-edited')
    expect(screen.getByText('Hand edit saved while Enhance ran')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Replace hand-edited description' }))
    expect(actions.forceAccept).toHaveBeenCalledOnce()
  })
})
