import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { expect, it, vi } from 'vitest'
import { useSaveNarrativeWorld } from './narrativeWorld'
import { applyCommand, useUndoStack } from '../undo'
import type { WorldDocument } from '../api/narrativeWorld'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
it('keeps exact World undo and redo guards across a document version upgrade', async () => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  const before: WorldDocument = {
    schema_version: 1,
    facts: [],
    knowledge: [],
    relationships: [],
    events: [],
    quests: [],
    restrictions: [],
  }
  let disk = before
  const changed: WorldDocument = {
    ...before,
    facts: [{ id: 'fact', name: 'Arrival', assertion: 'The ship arrived.', sources: [] }],
  }
  h.invoke.mockImplementation(async (command, args) => {
    if (command === 'narrative_world_save') disk = { ...args.document, schema_version: 2 }
    if (command === 'narrative_world_restore') {
      if (JSON.stringify(args.expected) !== JSON.stringify(disk)) throw new Error('CAS conflict')
      disk = { ...args.document, schema_version: 2 }
    }
    return { document: disk, stamp: { hash: 'current', size: 1, mtime_ms: 1 }, diagnostics: [] }
  })
  useUndoStack.setState({ projectId: 'project', past: [], future: [], busy: false })
  const client = new QueryClient()
  const { result } = renderHook(() => useSaveNarrativeWorld(), {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  })
  await act(async () => {
    await result.current.mutateAsync({
      file: { document: before, stamp: null, diagnostics: [] },
      document: changed,
    })
  })
  const entry = useUndoStack.getState().past[0]!
  await applyCommand(entry.undo[0]!)
  expect(disk).toEqual({ ...before, schema_version: 2 })
  await applyCommand(entry.redo[0]!)
  expect(disk).toEqual({ ...changed, schema_version: 2 })
  disk = { ...disk, facts: [] }
  await expect(applyCommand(entry.undo[0]!)).rejects.toThrow('CAS conflict')
})
