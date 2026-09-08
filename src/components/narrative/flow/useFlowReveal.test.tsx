import { act, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { useUI } from '../../../store/ui'
import { FlowOutline } from './FlowOutline'
import { resetFlowStore } from './flowStore'
import { sceneToFlow } from './source'
import { canonicalFlowActions, flowTarget } from './canonicalFlow'
import { NarrativeFlowPane } from '../NarrativeFlowPane'

const source = {
  id: 'scene',
  name: 'Ashfall',
  beats: [
    { id: 'arrival', title: 'Arrival' },
    { id: 'verdict', title: 'Verdict' },
  ],
}
const scene = sceneToFlow(source)
const actions = canonicalFlowActions(source, () => true, vi.fn(), { projectKey: 'ashfall' })
function outline(hidden = false) {
  return (
    <div hidden={hidden}>
      <FlowOutline scene={scene} onChange={vi.fn()} actions={actions} targetOf={flowTarget} />
    </div>
  )
}
beforeEach(() => {
  resetFlowStore()
  useUI.setState({
    narrativeReveal: null,
    narrative: { sceneId: null, beatId: null, lineId: null },
  })
})
it('keeps repeated hidden-tab reveals pending until the actual node becomes visible', async () => {
  const view = render(outline(true))
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'verdict' }, 'diagnostic', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  expect(document.activeElement).toBe(document.body)
  view.rerender(outline())
  const target = screen.getByRole('button', { name: 'BeatVerdict' })
  await waitFor(() => expect(target).toHaveFocus())
  screen.getByRole('button', { name: 'BeatArrival' }).focus()
  view.rerender(outline(true))
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'verdict' }, 'diagnostic', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  view.rerender(outline())
  await waitFor(() => expect(target).toHaveFocus())
})
it('does not steal focus for another project and falls back from a deleted route to its beat', async () => {
  render(outline())
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'verdict' }, 'diagnostic', {
      projectKey: 'elsewhere',
      focus: true,
    }),
  )
  expect(document.activeElement).toBe(document.body)
  act(() =>
    useUI
      .getState()
      .selectNarrative({ sceneId: 'scene', beatId: 'verdict', choiceId: 'removed' }, 'diagnostic', {
        projectKey: 'ashfall',
        focus: true,
      }),
  )
  await waitFor(() => expect(screen.getByRole('button', { name: 'BeatVerdict' })).toHaveFocus())
  expect(screen.getByRole('status')).toHaveTextContent('requested element is gone')
})
it('returns keyboard focus to the creation control after the final beat is deleted', async () => {
  render(
    <NarrativeFlowPane
      source={{ kind: 'ready', scene: sceneToFlow({ ...source, beats: [] }) }}
      actions={actions}
      onEdit={vi.fn()}
    />,
  )
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: null }, 'flow', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Add the first beat' })).toHaveFocus(),
  )
})

it('waits for a hidden target inside a visible outline before focusing it', async () => {
  render(outline())
  const target = screen.getByRole('button', { name: 'BeatVerdict' })
  target.hidden = true
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'verdict' }, 'diagnostic', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  expect(target).not.toHaveFocus()
  target.hidden = false
  await waitFor(() => expect(target).toHaveFocus())
})
it('does not consume a reveal when focusing the actual target fails', async () => {
  render(outline())
  const target = screen.getByRole('button', { name: 'BeatVerdict' })
  const focus = vi.spyOn(target, 'focus').mockImplementation(() => {})
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'verdict' }, 'diagnostic', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  expect(focus).toHaveBeenCalled()
  expect(target).not.toHaveFocus()
  focus.mockRestore()
  target.setAttribute('data-ready', 'true')
  await waitFor(() => expect(target).toHaveFocus())
})
it('waits for inherited disabled state to clear on the empty-scene creation control', async () => {
  render(
    <fieldset disabled>
      <NarrativeFlowPane
        source={{ kind: 'ready', scene: sceneToFlow({ ...source, beats: [] }) }}
        actions={actions}
        onEdit={vi.fn()}
      />
    </fieldset>,
  )
  const target = screen.getByRole('button', { name: 'Add the first beat' })
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', beatId: null }, 'flow', {
      projectKey: 'ashfall',
      focus: true,
    }),
  )
  expect(target).not.toHaveFocus()
  target.closest('fieldset')!.disabled = false
  await waitFor(() => expect(target).toHaveFocus())
})
