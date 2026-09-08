import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeTextLibrary } from './NarrativeTextLibrary'
import type { TextCatalog, TextFile } from '../../lib/api/narrativeText'

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

function draw(readOnly = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
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

  it('writes nothing at all in a read-only project', async () => {
    draw(true)
    const editor = await openEditor()
    expect(editor.getByLabelText('Name')).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Delete' })).toBeDisabled()
    expect(editor.getByRole('button', { name: 'Add entry' })).toBeDisabled()
  })
})
