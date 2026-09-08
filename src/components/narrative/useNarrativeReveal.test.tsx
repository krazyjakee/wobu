import { useRef } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { useUI } from '../../store/ui'
import { useNarrativeReveal } from './useNarrativeReveal'
function Editor({
  hidden = false,
  subsectionHidden = false,
  disabled = false,
}: {
  hidden?: boolean
  subsectionHidden?: boolean
  disabled?: boolean
}) {
  const root = useRef<HTMLDivElement>(null)
  useNarrativeReveal(root, '/project', 'scene')
  return (
    <div hidden={hidden}>
      <div ref={root}>
        <fieldset disabled={disabled} hidden={subsectionHidden}>
          <textarea aria-label="Wording" data-narrative-field="variant:variant" />
        </fieldset>
      </div>
    </div>
  )
}
it('waits for a hidden mounted editor, reveals repeated requests, and ignores another project', async () => {
  useUI.setState({ narrativeReveal: null })
  const view = render(<Editor hidden />)
  const wording = screen.getByLabelText('Wording')
  const request = () =>
    useUI
      .getState()
      .selectNarrative(
        { sceneId: 'scene', beatId: 'beat', lineId: 'slot', variantId: 'variant', field: 'text' },
        'search',
        { projectKey: '/project' },
      )
  act(request)
  expect(wording).not.toHaveFocus()
  view.rerender(<Editor hidden={false} />)
  await waitFor(() => expect(wording).toHaveFocus())
  fireEvent.blur(wording)
  act(() => {
    ;(wording as HTMLTextAreaElement).blur()
    request()
  })
  await waitFor(() => expect(wording).toHaveFocus())
  act(() => {
    ;(wording as HTMLTextAreaElement).blur()
    useUI.getState().selectNarrative({ sceneId: 'scene', variantId: 'variant' }, 'search', {
      projectKey: '/other',
    })
  })
  expect(wording).not.toHaveFocus()
})

it('waits for a hidden subsection and inherited disabled fieldset before acknowledging focus', async () => {
  useUI.setState({ narrativeReveal: null })
  const view = render(<Editor subsectionHidden disabled />)
  const wording = screen.getByLabelText('Wording')
  const scroll = vi.fn()
  wording.scrollIntoView = scroll
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', variantId: 'variant' }, 'diagnostic', {
      projectKey: '/project',
    }),
  )
  expect(scroll).not.toHaveBeenCalled()
  view.rerender(<Editor disabled />)
  await act(async () => {
    await new Promise((resolve) => requestAnimationFrame(resolve))
  })
  expect(wording).not.toHaveFocus()
  expect(scroll).not.toHaveBeenCalled()
  view.rerender(<Editor />)
  await waitFor(() => expect(wording).toHaveFocus())
  expect(scroll).toHaveBeenCalledOnce()
})
it('allows a scroll-only reveal of disabled readable content without focusing it', () => {
  useUI.setState({ narrativeReveal: null })
  render(<Editor disabled />)
  const wording = screen.getByLabelText('Wording')
  const scroll = vi.fn()
  wording.scrollIntoView = scroll
  act(() =>
    useUI.getState().selectNarrative({ sceneId: 'scene', variantId: 'variant' }, 'diagnostic', {
      projectKey: '/project',
      focus: false,
    }),
  )
  expect(scroll).toHaveBeenCalledOnce()
  expect(wording).not.toHaveFocus()
})
