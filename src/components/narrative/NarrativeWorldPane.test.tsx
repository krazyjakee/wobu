import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { qk } from '../../lib/queries/keys'
import { useUndoStack } from '../../lib/undo'
import { editorWrites, EditorWritesBlocked } from '../../lib/editorWrites'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import type { WorldFile, WorldDocument } from '../../lib/api/narrativeWorld'
import { NarrativeWorldPane } from './NarrativeWorldPane'
import { useWorldDrafts } from './worldDrafts'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
let saved: WorldFile
const base: WorldDocument = {
  schema_version: 1,
  facts: [
    {
      id: 'beacon',
      name: 'Beacon attack',
      assertion: 'The captain ordered the beacon burned.',
      sources: ['Harbour logbook'],
    },
  ],
  knowledge: [
    {
      id: 'kael_knows',
      name: 'Kael witnessed it',
      character: 'kael',
      fact: 'beacon',
      belief: 'true',
      provenance: 'witnessed',
      when: 'always',
    },
    {
      id: 'mira_knows',
      name: 'Mira was told',
      character: 'mira',
      fact: 'beacon',
      belief: 'true',
      provenance: { told: { by: 'kael' } },
      when: 'always',
    },
    {
      id: 'orren_rumour',
      name: 'Orren heard a rumour',
      character: 'orren',
      fact: 'beacon',
      belief: 'false',
      provenance: { rumour: { source: 'A dockside story' } },
      when: 'always',
    },
  ],
  relationships: [],
  events: [],
  quests: [],
  restrictions: [],
}
function mount(readOnly = false) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } },
  })
  qc.setQueryData(['narrative_world'], saved)
  qc.setQueryData(qk.nodes, [
    { id: 'kael', name: 'Kael', kind: 'character' },
    { id: 'mira', name: 'Mira', kind: 'character' },
    { id: 'orren', name: 'Orren', kind: 'character' },
    { id: 'harbour', name: 'Harbour', kind: 'setting' },
  ])
  qc.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
  qc.setQueryData(qk.narrativeState, {
    document: {
      schema_version: 1,
      variables: [{ name: 'has_logbook', type: 'bool', default: false, owner: 'narrative' }],
    },
    stamp: null,
  })
  return {
    qc,
    ...render(
      <QueryClientProvider client={qc}>
        <NarrativeWorldPane projectKey="/ashfall" readOnly={readOnly} />
      </QueryClientProvider>,
    ),
  }
}
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  resetNarrativeDraftGuards()
  useWorldDrafts.setState({ world: {}, variables: {} })
  useUndoStack.getState().setProject('world-test')
  useUndoStack.setState({ past: [], future: [] })
  saved = { document: structuredClone(base), stamp: null, diagnostics: [] }
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (cmd, args) => {
    if (cmd === 'narrative_world_get') return saved
    if (cmd === 'narrative_state_get')
      return { document: { schema_version: 1, variables: [] }, stamp: null }
    if (cmd === 'narrative_world_save') {
      saved = {
        document: args.document,
        stamp: { hash: 'saved', size: 100, mtime_ms: 0 },
        diagnostics: [],
      }
      return saved
    }
    if (cmd === 'narrative_scenes') return { scenes: [], unreadable: [] }
    if (cmd === 'narrative_diagnostics') return []
    return []
  })
})
afterEach(() => resetNarrativeDraftGuards())
describe('world authoring', () => {
  it('edits an attributed false belief without changing canonical fact, and records guarded undo', async () => {
    mount()
    fireEvent.click(screen.getByRole('button', { name: 'Knowledge' }))
    fireEvent.click(screen.getByRole('button', { name: 'Orren heard a rumour' }))
    expect(screen.getByLabelText('Belief')).toHaveValue('false')
    expect(screen.getByLabelText('Rumour source')).toHaveValue('A dockside story')
    fireEvent.change(screen.getByLabelText('Belief'), { target: { value: 'unknown' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save world' }))
    await waitFor(() => expect(saved.document.knowledge[2]!.belief).toBe('unknown'))
    expect(saved.document.facts).toEqual(base.facts)
    expect(useUndoStack.getState().past.at(-1)?.undo[0]).toEqual({
      type: 'worldRestore',
      document: base,
      expected: saved.document,
    })
    expect(screen.getByLabelText('Character').querySelector('option[value="harbour"]')).toBeNull()
  })
  it('shows reference impact before deleting canon and retains the claims for diagnostics', () => {
    mount()
    expect(screen.getByRole('heading', { name: 'Used by' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Delete record' }))
    expect(screen.getByRole('alert')).toHaveTextContent('3 records refer')
    expect(useWorldDrafts.getState().world['/ashfall']).toBeUndefined()
    fireEvent.click(screen.getByRole('button', { name: 'Delete referenced record' }))
    const draft = useWorldDrafts.getState().world['/ashfall']!.document
    expect(draft.facts).toEqual([])
    expect(draft.knowledge).toEqual(base.knowledge)
  })
  it('retains dirty world records and holds close even after the pane unmounts', async () => {
    const view = mount()
    fireEvent.change(screen.getByLabelText('Canonical assertion'), {
      target: { value: 'Corrected account' },
    })
    view.unmount()
    await expect(editorWrites.flushAll()).rejects.toBeInstanceOf(EditorWritesBlocked)
    mount()
    expect(screen.getByLabelText('Canonical assertion')).toHaveValue('Corrected account')
    fireEvent.click(screen.getByRole('button', { name: 'Discard world changes' }))
    await expect(editorWrites.flushAll()).resolves.toBeUndefined()
  })
  it('keeps a newer remounted draft after an earlier save resolves', async () => {
    let finish!: (value: WorldFile) => void
    h.invoke.mockImplementation((cmd, args) =>
      cmd === 'narrative_world_save'
        ? new Promise<WorldFile>((resolve) => {
            finish = resolve
            saved = { document: args.document, stamp: null, diagnostics: [] }
          })
        : Promise.resolve(saved),
    )
    const first = mount()
    fireEvent.change(screen.getByLabelText('Canonical assertion'), {
      target: { value: 'First edit' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save world' }))
    await waitFor(() => expect(finish).toBeDefined())
    first.unmount()
    mount()
    fireEvent.change(screen.getByLabelText('Canonical assertion'), {
      target: { value: 'Newer typing' },
    })
    await act(async () => finish(saved))
    expect(useWorldDrafts.getState().world['/ashfall']!.document.facts[0]!.assertion).toBe(
      'Newer typing',
    )
  })
  it('makes canonical forms read-only and keeps discard reachable for a retained draft', () => {
    useWorldDrafts
      .getState()
      .putWorld('/ashfall', { file: saved, document: { ...base, facts: [] } })
    mount(true)
    expect(screen.getByRole('button', { name: 'Add record' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Save world' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Discard world changes' })).toBeEnabled()
  })
})

it('does not round typed relationship integers or silently replace an empty value with zero', () => {
  saved.document.relationships = [
    {
      id: 'trust',
      name: 'Mira trusts Kael',
      from: 'mira',
      to: 'kael',
      kind: 'trust',
      value: 40,
      when: 'always',
    },
  ]
  mount()
  fireEvent.click(screen.getByRole('button', { name: 'Relationships' }))
  for (const value of ['', '9007199254740993', '9007199254740990.5', '1.5'])
    fireEvent.change(screen.getByLabelText('Relationship value'), { target: { value } })
  expect(screen.getByLabelText('Relationship value')).toHaveValue(40)
  expect(useWorldDrafts.getState().world['/ashfall']).toBeUndefined()
  fireEvent.change(screen.getByLabelText('Relationship value'), { target: { value: '-15' } })
  expect(useWorldDrafts.getState().world['/ashfall']?.document.relationships[0]?.value).toBe(-15)
})

it('blocks saves that would round a large integer inside an existing world condition', () => {
  saved.document.knowledge[0]!.when = {
    compare: { var: 'legacy_value', op: 'eq', value: { literal: Number('9223372036854775807') } },
  }
  mount()
  fireEvent.change(screen.getByLabelText('Canonical assertion'), {
    target: { value: 'Unrelated wording change' },
  })
  expect(screen.getByRole('button', { name: 'Save world' })).toBeDisabled()
  expect(
    screen.getByRole('button', { name: /outside the editor’s exact integer range/ }),
  ).toBeInTheDocument()
})
