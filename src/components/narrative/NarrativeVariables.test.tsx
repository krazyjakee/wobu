import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StateFile } from '../../lib/api'
import { qk } from '../../lib/queries/keys'
import { resetNarrativeDraftGuards } from '../../lib/narrativeDraftGuard'
import { NarrativeVariables } from './NarrativeVariables'
import { useWorldDrafts } from './worldDrafts'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
let saved: StateFile
function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } })
  qc.setQueryData(qk.narrativeState, saved)
  qc.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
  qc.setQueryData(['narrative_world'], {
    document: {
      schema_version: 1,
      facts: [],
      knowledge: [],
      relationships: [],
      events: [],
      quests: [],
      restrictions: [],
    },
    stamp: null,
    diagnostics: [],
  })
  return render(
    <QueryClientProvider client={qc}>
      <NarrativeVariables projectKey="variables" readOnly={false} />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  resetNarrativeDraftGuards()
  useWorldDrafts.setState({ variables: {}, world: {} })
  saved = {
    document: {
      schema_version: 1,
      variables: [{ name: 'trust', type: { int: { min: -100, max: 100 } }, default: 40 }],
    },
    stamp: { hash: 'old', size: 1, mtime_ms: 1 },
  }
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (name, args) => {
    if (name === 'narrative_state_save') {
      saved = { document: args.document, stamp: { hash: 'new', size: 2, mtime_ms: 2 } }
      return saved
    }
    if (name === 'narrative_state_get') return saved
    return {
      document: {
        facts: [],
        knowledge: [],
        relationships: [],
        events: [],
        quests: [],
        restrictions: [],
      },
    }
  })
})
afterEach(resetNarrativeDraftGuards)
describe('variable editing precision and preservation', () => {
  it('rejects empty, unsafe and fractional spelling before conversion can round it', async () => {
    mount()
    for (const label of ['Minimum', 'Maximum', 'Default value']) {
      const input = screen.getByLabelText(label)
      for (const value of ['', '9007199254740993', '9007199254740990.5', '1.5'])
        fireEvent.change(input, { target: { value } })
    }
    expect(screen.getByLabelText('Minimum')).toHaveValue(-100)
    expect(screen.getByLabelText('Maximum')).toHaveValue(100)
    expect(screen.getByLabelText('Default value')).toHaveValue(40)
    expect(useWorldDrafts.getState().variables.variables).toBeUndefined()
    fireEvent.change(screen.getByLabelText('Default value'), { target: { value: '-25' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save variables' }))
    await waitFor(() => expect(saved.document.variables[0]!.default).toBe(-25))
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_state_save',
      expect.objectContaining({
        expected: { kind: 'stamp', stamp: { hash: 'old', size: 1, mtime_ms: 1 } },
      }),
    )
  })

  it('blocks an unrelated save when loaded bounds are outside JavaScript’s exact range', () => {
    saved.document.variables[0]!.type = { int: { min: 0, max: Number('9223372036854775807') } }
    mount()
    fireEvent.change(screen.getByLabelText('Description'), {
      target: { value: 'Keep the original bounds' },
    })
    expect(screen.getByRole('button', { name: 'Save variables' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('existing file is preserved')
    expect(h.invoke.mock.calls.some(([name]) => name === 'narrative_state_save')).toBe(false)
  })

  it('keeps newer typing after a guarded save completes across remount', async () => {
    let finish!: (value: StateFile) => void
    h.invoke.mockImplementation((name) =>
      name === 'narrative_state_save'
        ? new Promise<StateFile>((resolve) => {
            finish = resolve
          })
        : Promise.resolve({
            document: {
              facts: [],
              knowledge: [],
              relationships: [],
              events: [],
              quests: [],
              restrictions: [],
            },
          }),
    )
    const view = mount()
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Submitted' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save variables' }))
    await waitFor(() => expect(finish).toBeDefined())
    view.unmount()
    mount()
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Newer typing' } })
    await act(async () => finish(saved))
    expect(useWorldDrafts.getState().variables.variables?.document.variables[0]?.description).toBe(
      'Newer typing',
    )
  })
})
