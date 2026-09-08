import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeTextLibrary } from './NarrativeTextLibrary'
import { useTextDrafts } from './textDrafts'
import type { TextCatalog, TextFile } from '../../lib/api/narrativeText'
import { invalidateNarrative } from '../../lib/queries/keys'
import { reviewFixture } from './review/reviewFixture.test-support'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const bark: TextFile = {
  asset: {
    id: 'bark',
    kind: 'bark',
    name: 'Gate guard',
    trigger: { event: 'player_passes_gate' },
    repeat: 'shuffle',
    policy: 'edited',
    participants: [{ entity: 'guard', role: 'guard' }],
    entries: [
      {
        id: 'entry',
        label: 'Move along',
        lines: [
          {
            id: 'line',
            speaker: { entity: 'guard' },
            variants: [{ id: 'variant', text: { revision: 'rev', body: 'Move along.' } }],
          },
        ],
      },
    ],
  },
  slug: 'gate-guard',
  rel: 'narrative/texts/gate-guard.yaml',
  stamp: { mtime_ms: 1, size: 2, hash: 'gate-hash' },
}

const catalog: TextCatalog = {
  assets: [
    { id: 'bark', kind: 'bark', name: 'Gate guard', slug: 'gate-guard', rel: bark.rel },
    {
      id: 'codex',
      kind: 'codex',
      name: 'The beacons',
      slug: 'the-beacons',
      rel: 'narrative/texts/the-beacons.yaml',
    },
  ],
  unreadable: [],
}

function respond(command: string, args: Record<string, unknown>): unknown {
  switch (command) {
    case 'narrative_texts':
      return catalog
    case 'narrative_text_get':
      return bark
    case 'narrative_text_diagnostics':
      return []
    case 'narrative_text_written':
      return { revision: 'sealed', body: args.body, provenance: 'human' }
    case 'narrative_text_create':
    case 'narrative_text_save':
      return bark
    default:
      return null
  }
}

/**
 * Open the first asset and wait for its document to arrive.
 *
 * The editor pane exists before the file does — the query is in flight — so a
 * synchronous lookup would race the read and pass or fail by timing.
 */
async function openEditor() {
  fireEvent.click(await screen.findByRole('button', { name: /Gate guard/ }))
  const editor = within(await screen.findByLabelText('Supporting text editor'))
  await editor.findByLabelText('Name')
  return editor
}

function draw(
  readOnly = false,
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
) {
  return render(
    <QueryClientProvider client={client}>
      <NarrativeTextLibrary
        projectKey="/world"
        readOnly={readOnly}
        nameOf={(entity) => (entity === 'guard' ? 'Seawall keeper' : undefined)}
        onClose={() => {}}
      />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  useTextDrafts.setState({ drafts: {} })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(respond(command, args ?? {})),
  )
})

describe('the Text library', () => {
  it('offers a template for each of the six supporting kinds', async () => {
    draw()
    const templates = await screen.findByLabelText('Template')
    const labels = Array.from(templates.querySelectorAll('option')).map((one) => one.textContent)
    expect(labels).toEqual([
      'Bark',
      'Ambient exchange',
      'Companion reaction',
      'Codex entry',
      'Quest summary',
      'Journal entry',
    ])
  })

  it('suggests a trigger that follows the chosen template', async () => {
    draw()
    const trigger = within(await screen.findByLabelText('Template').then((s) => s.closest('form')!))
    expect(trigger.getByLabelText('Trigger')).toHaveValue('player_passes')
    fireEvent.change(await screen.findByLabelText('Template'), { target: { value: 'codex' } })
    expect(trigger.getByLabelText('Trigger')).toHaveValue('codex_opened')
  })

  it('creates an asset with its kind, name and trigger', async () => {
    draw()
    const form = (await screen.findByLabelText('Template')).closest('form')!
    fireEvent.change(within(form).getByLabelText('Template'), { target: { value: 'journal' } })
    fireEvent.change(within(form).getByLabelText('Name'), { target: { value: 'Watch nights' } })
    fireEvent.submit(form)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('narrative_text_create', {
        kind: 'journal',
        name: 'Watch nights',
        event: 'day_ends',
      }),
    )
  })

  it('says the catalog is being read, and says so when it cannot be', async () => {
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      command === 'narrative_texts'
        ? Promise.reject(new Error('narrative/texts is not readable'))
        : Promise.resolve(respond(command, args ?? {})),
    )
    draw()
    // The in-flight state first: an empty pane while the read is running reads
    // as a project with no supporting text in it.
    expect(screen.getByText('Reading supporting text…')).toBeInTheDocument()
    expect(await screen.findByRole('alert')).toHaveTextContent('narrative/texts is not readable')
    expect(screen.queryByText('No supporting text yet. Choose a template above.')).toBeNull()
  })

  it('names a listed asset the editor cannot open', async () => {
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      command === 'narrative_text_get'
        ? Promise.reject(new Error('gate-guard.yaml stopped parsing'))
        : Promise.resolve(respond(command, args ?? {})),
    )
    draw()
    fireEvent.click(await screen.findByRole('button', { name: /Gate guard/ }))
    const editor = within(screen.getByLabelText('Supporting text editor'))
    expect(await editor.findByRole('alert')).toHaveTextContent('gate-guard.yaml stopped parsing')
    // "Select an asset" is advice the writer has already followed.
    expect(editor.queryByText('Select an asset to edit it.')).toBeNull()
  })

  it('lists every asset with the kind it is', async () => {
    draw()
    const chosen = await screen.findByRole('button', { name: /Gate guard/ })
    expect(chosen).toHaveTextContent('Bark')
    expect(screen.getByRole('button', { name: /The beacons/ })).toHaveTextContent('Codex entry')
  })

  it('opens an asset and shows its wording, trigger and selection policy', async () => {
    draw()
    const editor = await openEditor()
    expect(editor.getByLabelText('Name')).toHaveValue('Gate guard')
    expect(editor.getByLabelText('Trigger')).toHaveValue('player_passes_gate')
    expect(editor.getByLabelText('Selection')).toHaveValue('shuffle')
    expect(editor.getByLabelText('Entry 1, line 1')).toHaveValue('Move along.')
    // The speaker is named where a name exists, rather than showing a ULID.
    expect(editor.getByText('Seawall keeper')).toBeInTheDocument()
  })

  it('re-seals only the wording that changed, and never invents a revision here', async () => {
    draw()
    const editor = await openEditor()
    fireEvent.change(editor.getByLabelText('Entry 1, line 1'), {
      target: { value: 'Move along, quickly.' },
    })
    fireEvent.click(editor.getByRole('button', { name: 'Save' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('narrative_text_written', {
        body: 'Move along, quickly.',
        locked: false,
      }),
    )
    const save = h.invoke.mock.calls.find(([command]) => command === 'narrative_text_save')!
    const [, args] = save as [string, { asset: TextFile['asset']; expected: unknown }]
    const sealed = args.asset.entries?.[0]?.lines?.[0]?.variants?.[0]?.text
    expect(sealed?.revision).toBe('sealed')
    // The precondition is the stamp the document was read at, so a stale tab
    // cannot overwrite a newer file.
    expect(args.expected).toEqual({ kind: 'stamp', stamp: bark.stamp })
  })

  it('does not offer Save until something has actually changed', async () => {
    draw()
    const editor = await openEditor()
    expect(editor.getByRole('button', { name: 'Save' })).toBeDisabled()
    fireEvent.change(editor.getByLabelText('Name'), { target: { value: 'Seawall keeper' } })
    expect(editor.getByRole('button', { name: 'Save' })).toBeEnabled()
  })

  it('offers extra lines only for the kinds whose delivery is a sequence', async () => {
    draw()
    const editor = await openEditor()
    // A bark is one line by definition, so the form does not offer a second.
    expect(editor.queryByRole('button', { name: 'Add line' })).toBeNull()
    expect(editor.getByRole('button', { name: 'Add entry' })).toBeInTheDocument()
  })

  it('shows the backend’s own diagnostics rather than restating them', async () => {
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      Promise.resolve(
        command === 'narrative_text_diagnostics'
          ? [
              {
                kind: 'textAsset',
                code: 'no_text_entries',
                message: 'this text asset has no entries, so its trigger has nothing to deliver',
                destination: false,
              },
            ]
          : respond(command, args ?? {}),
      ),
    )
    draw()
    fireEvent.click(await screen.findByRole('button', { name: /Gate guard/ }))
    const problems = within(await screen.findByLabelText('Supporting text diagnostics'))
    expect(await problems.findByText(/nothing to deliver/)).toBeInTheDocument()
  })

  it('distinguishes pending and failed checks from a checked draft with no problems', async () => {
    let finish!: (value: unknown[]) => void
    let fail!: (error: Error) => void
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      command === 'narrative_text_diagnostics'
        ? new Promise((resolve, reject) => {
            finish = resolve
            fail = reject
          })
        : Promise.resolve(respond(command, args ?? {})),
    )
    draw()
    const editor = await openEditor()
    const problems = within(screen.getByLabelText('Supporting text diagnostics'))
    expect(problems.getByText('Saved text diagnostics')).toBeInTheDocument()
    expect(problems.getByText('Checking supporting text…')).toBeInTheDocument()
    expect(problems.queryByText('0 problems')).toBeNull()
    finish([])
    expect(await problems.findByText('0 problems')).toBeInTheDocument()

    fireEvent.change(editor.getByLabelText('Entry 1, line 1'), {
      target: { value: 'Draft wording.' },
    })
    expect(problems.getByText('Draft diagnostics')).toBeInTheDocument()
    expect(problems.getByText('Checking supporting text…')).toBeInTheDocument()
    expect(problems.queryByText('0 problems')).toBeNull()
    await waitFor(() => {
      const check = h.invoke.mock.calls
        .filter(([command]) => command === 'narrative_text_diagnostics')
        .at(-1)!
      expect(check[1].asset.entries[0].lines[0].variants[0].text.body).toBe('Draft wording.')
    })
    fail(new Error('Project source unavailable'))
    expect(await problems.findByRole('alert')).toHaveTextContent(
      'Could not check supporting text: Project source unavailable',
    )
    expect(problems.queryByText('0 problems')).toBeNull()
  })

  it('writes nothing at all in a read-only project', async () => {
    draw(true)
    const editor = await openEditor()
    expect(editor.getByLabelText('Name')).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Delete' })).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Add entry' })).toBeDisabled()
  })

  it('explains the chosen saved text line and refreshes affectedness after a source save', async () => {
    const source = structuredClone(bark)
    source.asset.entries!.push({
      id: 'second-entry',
      label: 'Second response',
      lines: [{ ...source.asset.entries![0]!.lines![0]!, id: 'second-line' }],
    })
    let changed = true
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) => {
      if (command === 'narrative_affected') {
        return Promise.resolve(
          changed
            ? ['line', 'second-line'].map((slot) => ({
                scene: null,
                asset: 'bark',
                slot,
                variant: 'variant',
                kind: 'changed',
                before: 'before',
                after: 'after',
                explanations: [
                  {
                    source: `text/bark/${slot}/when`,
                    context: 'entry condition',
                    line: slot,
                    message: `${slot} condition was edited.`,
                  },
                ],
              }))
            : [],
        )
      }
      if (command === 'narrative_text_get' || command === 'narrative_text_save')
        return Promise.resolve(source)
      return Promise.resolve(respond(command, args ?? {}))
    })
    draw()
    const editor = await openEditor()
    fireEvent.click(editor.getByRole('button', { name: 'Context' }))
    expect(await editor.findByText('line condition was edited.')).toBeInTheDocument()
    expect(editor.queryByText('second-line condition was edited.')).toBeNull()
    fireEvent.change(editor.getByLabelText('Context line'), { target: { value: 'second-line' } })
    expect(editor.getByText('second-line condition was edited.')).toBeInTheDocument()
    expect(editor.queryByText('line condition was edited.')).toBeNull()

    changed = false
    fireEvent.change(editor.getByLabelText('Name'), { target: { value: 'New name' } })
    fireEvent.click(editor.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(editor.queryByRole('region', { name: 'Why affected' })).toBeNull())
    expect(
      h.invoke.mock.calls.filter(([command]) => command === 'narrative_affected'),
    ).toHaveLength(2)
  })
})

describe('supporting text production controls', () => {
  it('refreshes an open Text Review when narrative sources are invalidated externally', async () => {
    const review = reviewFixture()
    review.scene_id = 'bark'
    review.lines = [
      {
        ...review.lines[0]!,
        target: { scene: 'bark', beat: 'entry', slot: 'line', variant: 'variant' },
        freshness: 'current',
      },
    ]
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      Promise.resolve(
        command === 'narrative_review_get' ? structuredClone(review) : respond(command, args ?? {}),
      ),
    )
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    draw(false, client)
    fireEvent.click((await openEditor()).getByRole('button', { name: 'Review' }))
    const queue = within(await screen.findByRole('region', { name: 'Project review queue' }))
    expect(await queue.findByText(/· Current$/)).toBeInTheDocument()

    review.lines[0]!.freshness = 'out_of_date'
    invalidateNarrative(client)
    expect(await queue.findByText(/· Out of date$/)).toBeInTheDocument()
    expect(queue.queryByText(/· Current$/)).toBeNull()
    expect(
      h.invoke.mock.calls.filter(([command]) => command === 'narrative_review_get'),
    ).toHaveLength(2)
  })

  it('pages a large catalog and combines search with type filters', async () => {
    const assets = Array.from({ length: 76 }, (_, index) => ({
      ...catalog.assets[index % 2]!,
      id: `asset-${index}`,
      name: `Harbor ${String(index).padStart(3, '0')}`,
    }))
    h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
      Promise.resolve(
        command === 'narrative_texts' ? { assets, unreadable: [] } : respond(command, args ?? {}),
      ),
    )
    const { container } = draw()
    await screen.findByText('76 matching of 76 assets')
    expect(container.querySelectorAll('[data-text-asset]')).toHaveLength(25)
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }))
    expect(screen.getByText('Page 2 of 4')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Search text library'), {
      target: { value: 'Harbor 00' },
    })
    expect(screen.getByText('10 matching of 76 assets')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Content type'), { target: { value: 'codex' } })
    expect(screen.getByText('5 matching of 76 assets')).toBeInTheDocument()
    expect(container.querySelectorAll('[data-text-asset]')).toHaveLength(5)
  })

  it('edits trigger and entry conditions and gates production controls while dirty', async () => {
    draw()
    const editor = await openEditor()
    expect(editor.getByRole('button', { name: 'Generate' })).toBeEnabled()
    fireEvent.change(editor.getByLabelText('Trigger condition rule'), {
      target: { value: 'never' },
    })
    fireEvent.change(editor.getByLabelText('Entry 1 condition rule'), {
      target: { value: 'never' },
    })
    expect(editor.getByRole('button', { name: 'Generate' })).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Review' })).toBeDisabled()
    fireEvent.click(editor.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'narrative_text_save',
        expect.objectContaining({
          asset: expect.objectContaining({
            trigger: { event: 'player_passes_gate', when: 'never' },
            entries: [expect.objectContaining({ when: 'never' })],
          }),
          expected: { kind: 'stamp', stamp: bark.stamp },
        }),
      ),
    )
  })

  it('opens the shared generation planner with stable asset and entry targets', async () => {
    draw()
    const editor = await openEditor()
    fireEvent.click(editor.getByRole('button', { name: 'Generate' }))
    expect(await screen.findByRole('heading', { name: 'Generate dialogue' })).toBeInTheDocument()
    expect(screen.getByText(/Move along.*Move along/)).toBeInTheDocument()
  })
})

it('retains a dirty asset across selection and library remount with its original stale save guard', async () => {
  const codex = {
    ...bark,
    asset: { ...bark.asset, id: 'codex', kind: 'codex' as const, name: 'The beacons' },
    stamp: { ...bark.stamp!, hash: 'codex-hash' },
  }
  let incoming = bark
  h.invoke.mockImplementation((command: string, args: Record<string, unknown>) =>
    Promise.resolve(
      command === 'narrative_text_get'
        ? args.assetId === 'codex'
          ? codex
          : incoming
        : respond(command, args ?? {}),
    ),
  )
  const first = draw()
  const editor = await openEditor()
  fireEvent.change(editor.getByLabelText('Entry 1, line 1'), {
    target: { value: 'My retained sentence.' },
  })
  fireEvent.click(screen.getByRole('button', { name: /Codex entry.*The beacons/ }))
  await waitFor(() =>
    expect(
      screen
        .getAllByLabelText('Name')
        .some((element) => (element as HTMLInputElement).value === 'The beacons'),
    ).toBe(true),
  )
  fireEvent.click(screen.getByRole('button', { name: /Bark.*Gate guard/ }))
  expect(await screen.findByLabelText('Entry 1, line 1')).toHaveValue('My retained sentence.')
  first.unmount()
  incoming = {
    ...bark,
    stamp: { ...bark.stamp!, hash: 'peer-change' },
    asset: { ...bark.asset, summary: 'A peer changed the context.' },
  }
  draw()
  const restored = await openEditor()
  expect(restored.getByLabelText('Entry 1, line 1')).toHaveValue('My retained sentence.')
  expect(restored.getByRole('alert')).toHaveTextContent('Your draft is retained')
  fireEvent.click(restored.getByRole('button', { name: 'Save' }))
  await waitFor(() =>
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_text_save',
      expect.objectContaining({ expected: { kind: 'stamp', stamp: bark.stamp } }),
    ),
  )
})
