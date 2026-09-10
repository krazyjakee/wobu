import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { qk } from '../../lib/queries/keys'
import { useUI } from '../../store/ui'
import { NarrativeInspector } from './NarrativeInspector'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

beforeEach(() => {
  useUI.setState({ narrative: { sceneId: 'council', beatId: 'evidence', lineId: 'line' } })
})

it('resolves saved context and keeps variant conditions and editorial dimensions separate', () => {
  const qc = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity } } })
  qc.setQueryData(qk.nodes, [{ id: 'mira', name: 'Mira' }])
  qc.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
  qc.setQueryData(qk.narrativeScene('council'), {
    scene: {
      id: 'council',
      name: 'Council hearing',
      participants: [{ entity: 'mira', role: 'witness' }],
      beats: [
        {
          id: 'evidence',
          title: 'Present evidence',
          intents: [
            { subject: { entity: 'mira' }, intent: 'Support Kael without naming her source' },
          ],
          must_not_reveal: ['Mira was inside the beacon house'],
          dialogue: [
            {
              id: 'line',
              speaker: { entity: 'mira' },
              policy: 'locked',
              variants: [
                {
                  id: 'high-trust',
                  when: { compare: { var: 'trust', op: 'ge', value: { literal: 40 } } },
                  text: {
                    body: 'I believe Kael.',
                    lifecycle: { policy: 'locked', review: 'approved', freshness: 'out_of_date' },
                  },
                },
              ],
            },
          ],
        },
      ],
    },
  })
  render(
    <QueryClientProvider client={qc}>
      <NarrativeInspector />
    </QueryClientProvider>,
  )
  expect(screen.getByText('Mira · witness')).toBeInTheDocument()
  expect(screen.getByText('Mira was inside the beacon house')).toBeInTheDocument()
  expect(screen.getByText('I believe Kael.')).toBeInTheDocument()
  expect(screen.getByText('locked · approved · out of date')).toBeInTheDocument()
  expect(screen.getByText(/trust/)).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Scene Council hearing' }))
  expect(useUI.getState().narrative).toEqual({ sceneId: 'council', beatId: null, lineId: null })
  expect(screen.queryByText('I believe Kael.')).not.toBeInTheDocument()
})
