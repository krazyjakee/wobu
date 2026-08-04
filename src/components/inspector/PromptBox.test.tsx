import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { PromptBox } from './PromptBox'
import type { CompiledPrompt, InfluenceFragment, ProjectSummary } from '../../lib/api'
import type { PromptOptions } from '../../lib/queries'
import { summary } from '../../test/fixtures'

/*
 * The compiled prompt box, whose whole promise is that nothing is hidden: the
 * string on screen is the string the backend compiled, every run of it says
 * where it came from, and everything left out is on screen with the reason.
 * The assertions below are about what a person can read, select, copy and click
 * — not about markup — because every one of these failures is silent. A prompt
 * that is subtly not the prompt, a fragment that vanished without a report, or a
 * mood board reference surfacing in a string somebody then pastes into a
 * provider's box all look fine until they are expensive.
 */

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const project: ProjectSummary = {
  id: 'p1',
  name: 'Ashfall',
  path: '/tmp/ashfall',
  onNetworkShare: false,
  readOnly: false,
  lastOpenedAt: null,
}

const kael = summary({ id: 'kael', name: 'Kael' })

function frag(over: Partial<InfluenceFragment> = {}): InfluenceFragment {
  return {
    layer: 'subject',
    nodeId: 'kael',
    sourceName: 'Kael',
    section: 'silhouette',
    text: 'tall and stooped',
    assetId: null,
    weight: 1,
    target: 'prompt',
    sendable: true,
    ...over,
  }
}

const style = frag({
  layer: 'style',
  nodeId: 'style-guide',
  sourceName: 'Ashfall Style',
  section: 'medium',
  text: 'ink wash',
  weight: 0.8,
})

function compiled(over: Partial<CompiledPrompt> = {}): CompiledPrompt {
  return {
    subjectId: 'kael',
    preset: {
      id: 'character_sheet',
      label: 'Character sheet',
      kinds: [],
      defaultFor: [],
      priorities: [],
      framing: '',
      aspect: '3:4',
      images: 4,
      views: [],
      imageConstraints: null,
    },
    prompt: '',
    negative: '',
    spans: [],
    dropped: [],
    overflow: null,
    ...over,
  }
}

/** What `prompt_compile` answers. Overwritten per test. */
let answer: (args: Record<string, unknown> | undefined) => CompiledPrompt = () => compiled()

const jump = vi.fn()
const writeText = vi.fn<(text: string) => Promise<void>>()

beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  h.invoke.mockReset()
  h.invoke.mockImplementation((cmd: string, args: Record<string, unknown> | undefined) =>
    cmd === 'prompt_compile' ? Promise.resolve(answer(args)) : Promise.resolve(null),
  )
  jump.mockReset()
  writeText.mockReset()
  writeText.mockResolvedValue(undefined)
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
})

function show(
  props: Partial<{
    project: ProjectSummary | null
    subject: typeof kael | null
    options: PromptOptions
  }> = {},
) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <QueryClientProvider client={qc}>
      <PromptBox
        project={'project' in props ? (props.project ?? null) : project}
        subject={'subject' in props ? (props.subject ?? null) : kael}
        options={props.options}
        onJump={jump}
      />
    </QueryClientProvider>,
  )
  return {
    ...view,
    /** Re-render inside the same client, which is what a slider drag does. */
    again: (options: PromptOptions) =>
      view.rerender(
        <QueryClientProvider client={qc}>
          <PromptBox project={project} subject={kael} options={options} onJump={jump} />
        </QueryClientProvider>,
      ),
  }
}

/** The positive prompt's paragraph — the text a person would select and copy. */
function promptText(): HTMLElement {
  const chan = screen.getByText('Prompt').closest('.prompt-chan') as HTMLElement
  return chan.querySelector('.prompt-text') as HTMLElement
}

/**
 * A run of the prompt, found by the attribution it carries.
 *
 * `data-attribution` rather than the `title` this used to read: the attribution
 * is a real tooltip now (#129), so it is only in the accessible tree while one
 * is open — but the string itself is on the run at all times, which is what
 * makes "colour is never the only carrier" true rather than conditional.
 */
function fragment(attribution: string, within: ParentNode = document): HTMLElement {
  const found = within.querySelector(`[data-attribution="${attribution}"]`)
  if (!found) throw new Error(`no prompt run attributed to ${attribution}`)
  return found as HTMLElement
}

async function findFragment(attribution: string): Promise<HTMLElement> {
  await waitFor(() => fragment(attribution))
  return fragment(attribution)
}

describe('the compiled prompt', () => {
  it('shows the string the backend compiled, not a reassembly of its spans', async () => {
    answer = () => compiled({ prompt: 'ink wash, tall and stooped', spans: [style, frag()] })
    show()

    // Read off the DOM rather than off a span list: this is what a selection
    // drag picks up and what a copy puts on the clipboard, and it has to be the
    // prompt character for character, separators included.
    await waitFor(() => expect(promptText().textContent).toBe('ink wash, tall and stooped'))
  })

  it('attributes every fragment in words as well as in colour', async () => {
    // Layer colours sit close together by design and some readers cannot tell
    // them apart at all, so the tint is never the only carrier: each run says
    // its layer, source and section, and the readout repeats it on hover.
    answer = () => compiled({ prompt: 'ink wash, tall and stooped', spans: [style, frag()] })
    show()

    const run = await findFragment('Style · Ashfall Style · medium — weight 0.80')
    expect(run.textContent).toBe('ink wash')
    expect(fragment('Subject · Kael · silhouette — weight 1.00')).toBeTruthy()

    fireEvent.mouseEnter(run)
    expect(screen.getByText(/Style · Ashfall Style · medium — weight 0\.80/)).toBeTruthy()
  })

  it('copies the plain prompt rather than what is on screen', async () => {
    // The clipboard gets the backend's own string. Copying the rendered markup
    // could differ by a space nobody would ever trace back to this box.
    answer = () => compiled({ prompt: 'ink wash, tall and stooped', spans: [style, frag()] })
    show()

    fireEvent.click(await screen.findByLabelText('Copy the prompt'))
    expect(writeText).toHaveBeenCalledWith('ink wash, tall and stooped')
  })

  it('jumps to the node a fragment came from, and offers no jump where there is no node', async () => {
    const shot = frag({
      layer: 'shot',
      nodeId: null,
      sourceName: 'Character sheet',
      section: 'framing',
      text: 'full body',
    })
    answer = () => compiled({ prompt: 'ink wash, full body', spans: [style, shot] })
    show()

    fireEvent.click(await findFragment('Style · Ashfall Style · medium — weight 0.80'))
    expect(jump).toHaveBeenCalledWith('style-guide')

    // The Shot layer's text comes from the output preset, so there is nothing to
    // open. A link that goes nowhere is worse than no link.
    const framing = fragment('Shot · Character sheet · framing — weight 1.00')
    expect(framing.getAttribute('role')).toBeNull()
    fireEvent.click(framing)
    expect(jump).toHaveBeenCalledTimes(1)
  })

  it('gives the negative prompt the same treatment', async () => {
    // A word nobody remembers writing is exactly as puzzling in the negative
    // prompt, and the layer it came from is the answer there too.
    const no = frag({ target: 'negative', section: 'avoid', text: 'blurry', layer: 'world' })
    answer = () => compiled({ prompt: 'ink wash', negative: 'blurry', spans: [style, no] })
    show()

    const negative = (await screen.findByText('Negative')).closest('.prompt-chan') as HTMLElement
    expect(fragment('World · Kael · avoid — weight 1.00', negative).textContent).toBe('blurry')
    // And the negative's words stay out of the positive prompt.
    expect(promptText().textContent).toBe('ink wash')
  })

  it('names every source in prose when asked, for anyone the tint does not reach', async () => {
    answer = () => compiled({ prompt: 'ink wash, tall and stooped', spans: [style, frag()] })
    show()

    fireEvent.click(await screen.findByText('Show sources'))
    const rows = Array.from(document.querySelectorAll('.prompt-list .prompt-row'))
    expect(rows.length).toBe(2)
    const first = rows[0]?.textContent ?? ''
    expect(first).toContain('Style')
    expect(first).toContain('Ashfall Style')
    expect(first).toContain('medium')
    expect(first).toContain('ink wash')
  })
})

describe('nothing is hidden', () => {
  it('reports what was cut, and says something different for each reason', async () => {
    // A slider that is down and a prompt that is too long are two different
    // problems with two different controls. Told as one list, the advice would
    // be wrong half the time.
    answer = () =>
      compiled({
        prompt: 'ink wash',
        spans: [style],
        dropped: [
          { fragment: frag({ text: 'lopsided gait' }), reason: 'silenced' },
          { fragment: frag({ text: 'a very long aside about the weather' }), reason: 'budget' },
        ],
      })
    show()

    const silenced = (await screen.findByText('Turned down')).closest(
      '.prompt-drops',
    ) as HTMLElement
    expect(within(silenced).getByText(/lopsided gait/)).toBeTruthy()
    expect(within(silenced).getByText(/Raise it and these come straight back/)).toBeTruthy()

    const overBudget = screen.getByText('Did not fit').closest('.prompt-drops') as HTMLElement
    expect(within(overBudget).getByText(/a very long aside/)).toBeTruthy()
    expect(within(overBudget).getByText(/lightest fragments/)).toBeTruthy()
  })

  it('never shows a moodboard_only fragment, in the prompt or in the drop report', async () => {
    // Kept out at the link, the fragment, the budget and the bridge. This box is
    // the last surface before a human reads the string and pastes it somewhere,
    // so it is kept out here too rather than trusted to have been.
    const mood = frag({ text: 'her mother, from the photograph', sendable: false, weight: 0 })
    answer = () =>
      compiled({
        prompt: 'ink wash',
        spans: [style, mood],
        dropped: [{ fragment: mood, reason: 'silenced' }],
      })
    show()

    await findFragment('Style · Ashfall Style · medium — weight 0.80')
    expect(document.body.textContent).not.toContain('her mother')
    expect(screen.queryByText('Turned down')).toBeNull()
  })

  it('says how far over the budget a prompt went rather than looking merely short', async () => {
    answer = () => compiled({ prompt: 'ink wash', spans: [style], overflow: 240 })
    show()
    expect(await screen.findByText(/Over the character budget by 240/)).toBeTruthy()
  })
})

describe('the empty states', () => {
  it('says there is no project, and does not ask the backend to compile one', async () => {
    show({ project: null })
    expect(await screen.findByText('No project open.')).toBeTruthy()
    expect(h.invoke).not.toHaveBeenCalled()
  })

  it('says nothing is selected, which is not the same as nothing to say', async () => {
    show({ subject: null })
    expect(await screen.findByText('Nothing selected.')).toBeTruthy()
    expect(h.invoke).not.toHaveBeenCalled()
  })

  it('sends the user off to write when there is nothing upstream at all', async () => {
    answer = () => compiled()
    show()
    expect(await screen.findByText('Nothing to compile yet.')).toBeTruthy()
    expect(screen.getByText(/Kael has no described sections/)).toBeTruthy()
  })

  it('sends the user to the drop report when everything compiled to nothing', async () => {
    // Different from the state above and it must read differently: something was
    // written, and all of it lost. The report is the whole answer, so it stays.
    answer = () =>
      compiled({ dropped: [{ fragment: frag({ text: 'lopsided gait' }), reason: 'silenced' }] })
    show()

    expect(await screen.findByText(/Everything was left out/)).toBeTruthy()
    expect(screen.getByText('Turned down')).toBeTruthy()
    expect(screen.queryByText('Nothing to compile yet.')).toBeNull()
  })

  it('shows the error rather than compiling forever', async () => {
    // The backend's own sentence is "no node with id …"; what the box shows is
    // `lib/errorCopy.ts`'s translation of it (#127).
    h.invoke.mockRejectedValue({ code: 'node.not_found', message: 'no node with id 01ARZ3ND' })
    show()
    expect(await screen.findByText(/not in this project any more/)).toBeTruthy()
  })
})

describe('living updates', () => {
  it('recompiles when the controls move, and holds the last prompt while it does', async () => {
    // The weight sliders (#47) do not exist yet. This is the contract they will
    // land against: the box owns no copy of the prompt, so a new `options` is a
    // new query and a new string. `keepPreviousData` is what stops it blanking
    // under the cursor on every frame of a drag — a box that empties and refills
    // sixty times a second is unreadable and unusable.
    answer = (args) =>
      (args?.sliders as { value: number }[] | undefined)?.[0]?.value === 0
        ? compiled({ prompt: 'ink wash', spans: [style] })
        : compiled({ prompt: 'ink wash, tall and stooped', spans: [style, frag()] })

    const view = show({ options: { sliders: [{ nodeId: 'kael', value: 1 }] } })
    await waitFor(() => expect(promptText().textContent).toBe('ink wash, tall and stooped'))

    view.again({ sliders: [{ nodeId: 'kael', value: 0 }] })
    // The old prompt is still on screen for the frame the next one takes.
    expect(promptText().textContent).toBe('ink wash, tall and stooped')
    await waitFor(() => expect(promptText().textContent).toBe('ink wash'))

    expect(h.invoke).toHaveBeenLastCalledWith(
      'prompt_compile',
      expect.objectContaining({ sliders: [{ nodeId: 'kael', value: 0 }] }),
    )
  })

  it('reads the weight the prompt has now, not the one it had when the pointer arrived', async () => {
    // A drag moves the slider under a stationary cursor: no second hover ever
    // fires, so a readout holding the fragment it was handed would go on
    // reporting a weight the prompt no longer has for as long as the pointer
    // does not move — during exactly the interaction it exists to explain.
    answer = (args) => {
      const value = (args?.sliders as { value: number }[] | undefined)?.[0]?.value ?? 1
      return compiled({ prompt: 'ink wash', spans: [{ ...style, weight: value }] })
    }

    const view = show({ options: { sliders: [{ nodeId: 'style-guide', value: 0.8 }] } })
    fireEvent.mouseEnter(await findFragment('Style · Ashfall Style · medium — weight 0.80'))
    expect(screen.getByText(/weight 0\.80/)).toBeTruthy()

    view.again({ sliders: [{ nodeId: 'style-guide', value: 0.2 }] })
    await waitFor(() => expect(screen.getByText(/weight 0\.20/)).toBeTruthy())
  })

  it('does not recompile when nothing moved', async () => {
    // An omitted `options` must be the same key every render, or the Inspector
    // would recompile on every keystroke somewhere else in the app.
    answer = () => compiled({ prompt: 'ink wash', spans: [style] })
    const view = show()
    await waitFor(() => expect(promptText().textContent).toBe('ink wash'))
    view.again({})
    await waitFor(() => expect(promptText().textContent).toBe('ink wash'))
    expect(h.invoke.mock.calls.filter((c) => c[0] === 'prompt_compile').length).toBe(1)
  })
})
