import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { PreviewFrame } from '../../lib/api/narrativePreview'
import { useUI } from '../../store/ui'
import { NarrativePreviewPane } from './NarrativePreviewPane'
import { usePreviewSessions } from './previewStore'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
const line: PreviewFrame = {
  snapshot: { at: 'line' },
  current: {
    line: {
      scene: 'scene',
      beat: 'beat',
      slot: 'slot',
      variant: 'variant',
      speaker: 'player',
      text: 'I have proof.',
      revision: 'revision',
    },
  },
  state: { trust: 40 },
}
const choices: PreviewFrame = {
  snapshot: { at: 'choices' },
  current: {
    choices: { scene: 'scene', beat: 'beat', choices: [{ id: 'choice', label: 'Show proof' }] },
  },
  state: { trust: 40 },
}
const command: PreviewFrame = {
  snapshot: { at: 'command' },
  current: { game_command: { token: 'token', name: 'award_badge', args: [true] } },
  state: { trust: 50 },
}
const end: PreviewFrame = {
  snapshot: { at: 'end' },
  current: { end: { label: 'Supported' } },
  state: { trust: 50 },
}
function mount(projectKey = 'project') {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <NarrativePreviewPane projectKey={projectKey} />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  usePreviewSessions.setState({ sessions: {}, diagnostics: {}, busy: {} })
  useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'beat' }, 'script')
  useUI.getState().setNarrativeTab('preview')
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (name: string, args: Record<string, unknown>) => {
    if (name === 'narrative_state_get')
      return {
        document: {
          schema_version: 1,
          variables: [{ name: 'trust', type: { int: { min: 0, max: 100 } }, default: 40 }],
        },
        stamp: null,
      }
    if (name === 'node_list') return []
    if (name === 'narrative_compile') return { graph: { fingerprint: 'build' }, diagnostics: [] }
    if (name === 'narrative_preview_start') return line
    if (name === 'narrative_preview_step') {
      const action = args.action as { kind: string }
      if (action.kind === 'advance') return choices
      if (action.kind === 'choose') return command
      if (action.kind === 'completeCommand') return end
      if (action.kind === 'restore') return line
    }
    throw new Error(`Unexpected ${name}`)
  })
})
describe('deterministic narrative preview', () => {
  it('sets starting state, follows choices, manually acknowledges commands and restores a snapshot', async () => {
    mount()
    fireEvent.change(await screen.findByLabelText('Initial trust'), { target: { value: '71' } })
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    expect(await screen.findByText('I have proof.')).toBeInTheDocument()
    expect(h.invoke).toHaveBeenCalledWith('narrative_preview_start', {
      graph: { fingerprint: 'build' },
      sceneId: 'scene',
      initialState: { trust: 71 },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save snapshot' }))
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Show proof' }))
    expect(await screen.findByText('Host command: award_badge')).toBeInTheDocument()
    expect(screen.getByText('50')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Acknowledge command' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Scene ended: Supported')
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_preview_step',
      expect.objectContaining({ action: { kind: 'completeCommand', token: 'token' } }),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Restore snapshot' }))
    expect(await screen.findByText('I have proof.')).toBeInTheDocument()
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_preview_step',
      expect.objectContaining({ snapshot: { at: 'line' }, action: { kind: 'restore' } }),
    )
    expect(usePreviewSessions.getState().sessions['project:scene']?.trace).toHaveLength(5)
  })

  it('retains playback across tabs, isolates projects and links line identity to Script', async () => {
    const view = mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    await screen.findByText('I have proof.')
    view.unmount()
    const again = mount()
    expect(screen.getByText('I have proof.')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Open this line in Script' }))
    expect(useUI.getState().narrative).toMatchObject({
      sceneId: 'scene',
      beatId: 'beat',
      lineId: 'slot',
    })
    expect(useUI.getState().narrativeTab).toBe('script')
    again.unmount()
    mount('other-project')
    expect(screen.queryByText('I have proof.')).not.toBeInTheDocument()
  })

  it('blocks compilation failures and links diagnostics to their scene and beat', async () => {
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) =>
      name === 'narrative_compile'
        ? Promise.resolve({
            graph: null,
            diagnostics: [
              {
                scene: 'other-scene',
                site: { variant: { beat: 'bad-beat', slot: 'slot', variant: 'variant' } },
                severity: 'error',
                code: 'type_error',
                message: 'Unknown trust variable',
              },
            ],
          })
        : prior(name, args),
    )
    mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    fireEvent.click(await screen.findByRole('button', { name: 'error: Unknown trust variable' }))
    expect(useUI.getState().narrative).toMatchObject({ sceneId: 'other-scene', beatId: 'bad-beat' })
    expect(h.invoke.mock.calls.some(([name]) => name === 'narrative_preview_start')).toBe(false)
  })

  it('keeps a failed step and serializes operations across a tab remount', async () => {
    let reject: ((error: Error) => void) | undefined
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) =>
      name === 'narrative_preview_step'
        ? new Promise((_, fail) => {
            reject = fail
          })
        : prior(name, args),
    )
    const view = mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }))
    await waitFor(() => expect(reject).toBeDefined())
    view.unmount()
    mount()
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled()
    await act(async () => reject?.(new Error('Invalid choice state')))
    expect(screen.getByText('I have proof.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })

  it('requires valid explicit host signatures before starting', async () => {
    mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award.badge":["bool"]}' },
    })
    expect(screen.getByRole('button', { name: 'Start preview' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":[{"enum":{"members":["true"]}}]}' },
    })
    expect(screen.getByRole('button', { name: 'Start preview' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":["bool"]}' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    await screen.findByText('I have proof.')
    expect(h.invoke).toHaveBeenCalledWith('narrative_compile', {
      commands: { award_badge: ['bool'] },
    })
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":["float"]}' },
    })
    expect(screen.getByRole('button', { name: 'Restart preview' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('Argument types')
  })
})
