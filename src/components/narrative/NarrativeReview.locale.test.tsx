import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { NarrativeReview } from './NarrativeReview'
import { reviewFixture } from './review/reviewFixture.test-support'
const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
it('filters actual Review rows by locale freshness and opens its localisation workflow', async () => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  const view = reviewFixture()
  view.lines.push({
    ...view.lines[0]!,
    target: { ...view.lines[0]!.target, variant: 'other' },
    text: { body: 'Other line', revision: 'other' },
  })
  invoke.mockImplementation(async (command: string) => {
    if (command === 'narrative_review_list')
      return {
        scenes: [view],
        errors: [],
        next_offset: null,
        total_scenes: 1,
        catalog_revision: 'catalog',
      }
    if (command === 'narrative_generation_history') return []
    if (command === 'narrative_locale_get')
      return {
        policy: { version: 1, source: 'en', required: { fr: false } },
        policy_guard: 'policy',
        translation_guards: {},
        sources: {
          'warning-main': {
            id: 'warning-main',
            ready: true,
            guard: 'changed-source',
            revision: 'original-revision',
            text: 'The beacon failed.',
          },
        },
        translations: [
          {
            version: 1,
            locale: 'fr',
            variant_id: 'warning-main',
            history: [
              {
                source_revision: 'original-revision',
                source_guard: 'old-source',
                forms: { other: 'Le phare est éteint.' },
                approved: true,
                actor: 'Translator',
              },
            ],
          },
        ],
      }
    return null
  })
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <NarrativeReview
        projectKey="/world"
        readOnly={false}
        sceneName={() => 'Beacon'}
        speakerName={() => 'Mara'}
        onSource={() => {}}
        onClose={() => {}}
      />
    </QueryClientProvider>,
  )
  const queue = within(await screen.findByRole('region', { name: 'Project review queue' }))
  await queue.findByText('Other line')
  expect(queue.getAllByRole('checkbox')).toHaveLength(2)
  fireEvent.change(screen.getByLabelText('Translation readiness'), {
    target: { value: 'out_of_date' },
  })
  await queue.findByText('The beacon failed.')
  expect(queue.getAllByRole('checkbox')).toHaveLength(1)
  expect(queue.queryByText('Other line')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Localisation…' }))
  expect(await screen.findByRole('heading', { name: 'Localisation' })).toBeInTheDocument()
})
