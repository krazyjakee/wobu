import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { NarrativeLocale } from './NarrativeLocale'
import type { LocaleView } from '../../lib/api/narrativeLocale'
import { advanceProjectSession } from '../../lib/projectSession'
const h = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: h.save }))
let view: LocaleView
beforeEach(() => {
  h.invoke.mockReset()
  h.save.mockReset()
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  view = {
    policy: { version: 1, source: 'en', required: {} },
    policy_guard: 'policy-v1',
    translation_guards: {},
    translations: [],
    sources: {
      line: {
        id: 'line',
        slot: 'slot',
        container: 'asset',
        speaker: 'Mara',
        text: 'Hello {name}',
        revision: 'rev',
        guard: 'source-v1',
        context: 'Harbor',
        delivery_notes: 'Quiet',
        placeholders: ['name'],
        ready: true,
      },
    },
  }
  h.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_locale_get') return structuredClone(view)
    if (command === 'narrative_locale_preview') return []
    if (command === 'narrative_locale_import') {
      view.translations = [
        {
          version: 1,
          locale: 'fr',
          variant_id: 'line',
          history: [
            {
              source_revision: 'rev',
              source_guard: 'source-v1',
              forms: { other: 'مرحبا {name}\n"<script>"' },
              approved: false,
              actor: 'Translator',
            },
          ],
        },
      ]
      view.translation_guards['fr/line'] = 'translation-v1'
      return { applied: ['line'], diagnostics: [], conflicts: {} }
    }
    if (command === 'narrative_locale_approve') {
      view.translations[0]!.history[0]!.approved = true
      return null
    }
    if (command === 'narrative_locale_policy') {
      view.policy = args.policy as LocaleView['policy']
      return null
    }
    return 'csv rows'
  })
})
function draw(readOnly = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <NarrativeLocale projectKey="/world" readOnly={readOnly} onClose={() => {}} />
    </QueryClientProvider>,
  )
}
it('previews before guarded partial import, displays RTL safely and approves independently', async () => {
  draw()
  await screen.findByText('Hello {name}')
  expect(screen.getByRole('button', { name: 'Import eligible rows' })).toBeDisabled()
  fireEvent.change(screen.getByLabelText('Interchange contents'), {
    target: { value: 'returned csv' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Preview import' }))
  await screen.findByText(/No row problems/)
  fireEvent.click(screen.getByRole('button', { name: 'Import eligible rows' }))
  await screen.findByText(/1 rows imported/)
  const translation = screen.getByText('مرحبا {name} "<script>"')
  expect(translation.closest('[dir]')).toHaveAttribute('dir', 'auto')
  // Native dir=auto chooses the first strong character. A Latin category label
  // inside this paragraph would force Arabic wording into an LTR paragraph.
  expect(translation.tagName).toBe('P')
  expect(translation.textContent).toBe('مرحبا {name}\n"<script>"')
  expect(translation.querySelector('strong')).toBeNull()
  expect(translation.previousElementSibling).toHaveTextContent('other:')
  expect(document.querySelector('script')).toBeNull()
  fireEvent.click(screen.getByRole('button', { name: 'Approve translation' }))
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Approve translation' })).toBeDisabled(),
  )
  expect(h.invoke).toHaveBeenCalledWith(
    'narrative_locale_approve',
    expect.objectContaining({
      row: expect.objectContaining({
        translation_guard: 'translation-v1',
        source: expect.objectContaining({ guard: 'source-v1' }),
        forms: { other: 'مرحبا {name}\n"<script>"' },
      }),
    }),
  )
})
it('pages the whole catalog and combines search with translation status', async () => {
  const original = view.sources.line!
  view.sources = Object.fromEntries(
    Array.from({ length: 76 }, (_, i) => [
      `id-${i}`,
      { ...original, id: `id-${i}`, text: `Phrase ${i}` },
    ]),
  )
  draw()
  await screen.findByText('76 matching of 76 source strings')
  expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(26)
  fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
  expect(screen.getByText('Page 2 of 4')).toBeInTheDocument()
  fireEvent.change(screen.getByLabelText('Search localisation'), { target: { value: 'Phrase 75' } })
  expect(screen.getByText('1 matching of 76 source strings')).toBeInTheDocument()
  expect(screen.getByText('Phrase 75')).toBeInTheDocument()
  fireEvent.change(screen.getByLabelText('Translation status'), { target: { value: 'approved' } })
  expect(screen.getByText('0 matching of 76 source strings')).toBeInTheDocument()
})
it('saves explicit fallback through the original policy guard and blocks writes read-only', async () => {
  const first = draw()
  await screen.findByText('Hello {name}')
  fireEvent.click(screen.getByLabelText('Require this locale for Release'))
  await waitFor(() =>
    expect(screen.getByLabelText('Explicitly allow truncation and source fallback')).toBeEnabled(),
  )
  fireEvent.click(screen.getByLabelText('Explicitly allow truncation and source fallback'))
  await waitFor(() =>
    expect(h.invoke).toHaveBeenCalledWith('narrative_locale_policy', {
      policy: { version: 1, source: 'en', required: { fr: true } },
      expected: 'policy-v1',
    }),
  )
  first.unmount()
  draw(true)
  await screen.findByText('Hello {name}')
  expect(screen.getByLabelText('Require this locale for Release')).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Import eligible rows' })).toBeDisabled()
})
it('does not export into a different project after a delayed destination chooser', async () => {
  let choose!: (path: string) => void
  h.save.mockImplementation(
    () =>
      new Promise((resolve) => {
        choose = resolve
      }),
  )
  draw()
  await screen.findByText('Hello {name}')
  fireEvent.click(screen.getByRole('button', { name: 'Export locale…' }))
  advanceProjectSession()
  choose('/tmp/other.csv')
  await Promise.resolve()
  await Promise.resolve()
  expect(h.invoke.mock.calls.some(([command]) => command === 'narrative_locale_export')).toBe(false)
})
