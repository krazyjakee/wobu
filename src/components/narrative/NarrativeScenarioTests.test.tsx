import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { NarrativeScenarioTests } from './NarrativeScenarioTests'
import { PreviewScenarios } from './PreviewScenarios'
import type { ScenarioFile } from '../../lib/api/narrativeScenarios'
const api = vi.hoisted(() => ({ list: vi.fn(), run: vi.fn(), save: vi.fn() }))
vi.mock('../../lib/api/narrativeScenarios', () => ({
  narrativeScenariosList: api.list,
  narrativeScenarioRun: api.run,
  narrativeScenarioSave: api.save,
}))
const file: ScenarioFile = {
  id: 'scenario',
  name: 'High trust / witnessed',
  stamp: { hash: 'original' } as unknown as ScenarioFile['stamp'],
  scenario: {
    version: 1,
    scene: 'scene',
    initial_state: { trust: 70 },
    seed: 17,
    commands: { log_watch: [] },
    steps: [{ action: null, expect: { state: { trust: 70 } } }],
  },
}
const site = {
  scene: 'scene',
  beat: 'beat',
  choice: 'choice',
  outcome: null,
  slot: null,
  variant: null,
}
beforeEach(() => {
  vi.resetAllMocks()
  api.list.mockResolvedValue([file])
  api.save.mockResolvedValue(file)
})
describe('scenario author controls', () => {
  it('runs saved scenarios and opens the first divergent source', async () => {
    const onSource = vi.fn()
    const onClose = vi.fn()
    api.run.mockResolvedValue({
      id: file.id,
      diagnostics: [],
      report: {
        passed: false,
        checked_steps: 3,
        divergence: {
          step: 3,
          field: 'state.trust',
          expected: 75,
          actual: 76,
          site,
          trace: { committed: true, records: [], omitted: 0, error: null },
        },
      },
    })
    render(<NarrativeScenarioTests readOnly={false} onSource={onSource} onClose={onClose} />)
    await screen.findByText(file.name)
    fireEvent.click(screen.getByRole('button', { name: 'Run all scenarios' }))
    expect(await screen.findByText('Failed at step 4: state.trust')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Open first divergence in Script' }))
    expect(onSource).toHaveBeenCalledWith(site)
    expect(onClose).toHaveBeenCalled()
  })
  it('saves partial assertions with the original source stamp and retains a conflicting draft', async () => {
    api.save.mockRejectedValue({ message: 'Another writer changed this scenario' })
    render(<NarrativeScenarioTests readOnly={false} onSource={vi.fn()} onClose={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: `Edit assertions for ${file.name}` }))
    const partial = {
      ...file.scenario,
      steps: [{ action: null, expect: { state: { trust: 71 } } }],
    }
    fireEvent.change(screen.getByLabelText('Scenario source (JSON)'), {
      target: { value: JSON.stringify(partial) },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save assertions' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Another writer changed this scenario',
    )
    expect(api.save).toHaveBeenCalledWith(file.name, JSON.stringify(partial), file)
    expect(screen.getByLabelText('Scenario source (JSON)')).toHaveValue(JSON.stringify(partial))
  })
  it('saves complete tapes, loads captured input/signatures and refuses incomplete recording', async () => {
    const onLoad = vi.fn().mockResolvedValue(undefined)
    const view = render(
      <PreviewScenarios
        sceneId="scene"
        busy={false}
        readOnly={false}
        tape={{ scenario: file.scenario, incomplete: false }}
        onLoad={onLoad}
      />,
    )
    fireEvent.click(screen.getByText('Saved scenarios'))
    fireEvent.change(screen.getByLabelText('Scenario name'), { target: { value: 'Saved watch' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save scenario' }))
    await waitFor(() => expect(api.save).toHaveBeenCalledWith('Saved watch', file.scenario))
    fireEvent.click(screen.getByRole('button', { name: 'Browse scenarios' }))
    await screen.findByRole('option', { name: file.name })
    fireEvent.change(screen.getByLabelText('Saved scenario'), { target: { value: file.id } })
    fireEvent.click(screen.getByRole('button', { name: 'Load scenario' }))
    await waitFor(() => expect(onLoad).toHaveBeenCalledWith(file.scenario))
    view.rerender(
      <PreviewScenarios
        sceneId="scene"
        busy={false}
        readOnly={false}
        tape={{ scenario: file.scenario, incomplete: true }}
        onLoad={onLoad}
      />,
    )
    expect(screen.getByRole('button', { name: 'Save scenario' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('tape is incomplete')
  })
})
