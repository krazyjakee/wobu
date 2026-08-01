import { describe, expect, it } from 'vitest'
import type { CompiledPrompt, FragmentTarget, InfluenceFragment, InfluenceLayer } from './api'
import { dropGroups, fragmentLabel, promptEmptiness, promptSegments, sectionLabel } from './prompt'

/*
 * The prompt box's arithmetic. Every test here guards the same promise from a
 * different side: what is on screen is the string the backend compiled, and
 * everything the backend left out is accounted for. The two ways to break that
 * are showing text nobody compiled and quietly dropping text somebody wrote.
 */

function frag(over: Partial<InfluenceFragment> = {}): InfluenceFragment {
  return {
    layer: 'subject' as InfluenceLayer,
    nodeId: 'kael',
    sourceName: 'Kael',
    section: 'silhouette',
    text: 'tall and stooped',
    assetId: null,
    weight: 1,
    target: 'prompt' as FragmentTarget,
    sendable: true,
    ...over,
  }
}

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
    },
    prompt: '',
    negative: '',
    spans: [],
    dropped: [],
    overflow: null,
    ...over,
  }
}

/** Every segment, joined, is what the user copies — so it must be the prompt. */
function joined(segments: { text: string }[]): string {
  return segments.map((s) => s.text).join('')
}

describe('promptSegments', () => {
  it('attributes each run of the compiled string to the fragment that put it there', () => {
    const a = frag({ layer: 'style', sourceName: 'Ashfall Style', text: 'ink wash' })
    const b = frag({ text: 'tall and stooped' })
    const text = 'ink wash, tall and stooped'

    const segments = promptSegments(text, [a, b], 'prompt')

    expect(segments.map((s) => s.text)).toEqual(['ink wash', ', ', 'tall and stooped'])
    expect(segments.map((s) => s.fragment)).toEqual([a, null, b])
    expect(joined(segments)).toBe(text)
  })

  it('keeps the two channels apart, though one list carries both', () => {
    // `prompt_spans` returns both prompts interleaved in reading order, which is
    // also emission order within each pool. Rendering the positive prompt from
    // the unfiltered list would tint it with the negative prompt's words.
    const yes = frag({ text: 'ink wash' })
    const no = frag({ text: 'blurry', target: 'negative', section: 'avoid' })

    expect(promptSegments('ink wash', [yes, no], 'prompt').map((s) => s.text)).toEqual(['ink wash'])
    expect(promptSegments('blurry', [yes, no], 'negative').map((s) => s.text)).toEqual(['blurry'])
  })

  it('never renders a moodboard_only fragment, whatever else it says about itself', () => {
    // The one thing in Wobu that is shown to the human and sent nowhere. It has
    // been kept out at the link, the fragment, the budget and the bridge; this
    // is the last surface before someone reads the prompt and copies it by hand
    // into a provider's box.
    const mood = frag({ text: 'her mother, from the photograph', sendable: false })
    expect(promptSegments('', [mood], 'prompt')).toEqual([])

    // And not even if a future bridge routes one at the prompt while leaving
    // `sendable` false — the two say different things and the stricter wins.
    const mislabelled = frag({ text: 'private', sendable: false, target: 'prompt' })
    const segments = promptSegments('ink wash', [mislabelled, frag({ text: 'ink wash' })], 'prompt')
    expect(joined(segments)).toBe('ink wash')
    expect(segments.some((s) => s.text.includes('private'))).toBe(false)
  })

  it('shows the compiled string untinted rather than a plausible reassembly of it', () => {
    // If the spans and the string ever disagree — a separator change, a routing
    // change, a fragment rewritten between compiling and emitting — the box
    // loses its colours and keeps its honesty. Reassembling from the spans would
    // put a string on screen that no generation would ever send.
    const stale = frag({ text: 'ink wash' })
    const segments = promptSegments('charcoal, tall and stooped', [stale], 'prompt')

    expect(segments).toEqual([{ text: 'charcoal, tall and stooped', fragment: null }])
    expect(joined(segments)).toBe('charcoal, tall and stooped')
  })

  it('does not silently swallow a tail the spans do not account for', () => {
    const segments = promptSegments('ink wash, and more', [frag({ text: 'ink wash' })], 'prompt')
    expect(joined(segments)).toBe('ink wash, and more')
    expect(segments).toEqual([{ text: 'ink wash, and more', fragment: null }])
  })

  it('has nothing to render for an empty string', () => {
    expect(promptSegments('', [], 'prompt')).toEqual([])
  })
})

describe('dropGroups', () => {
  it('keeps the two reasons apart, because they send the user to two controls', () => {
    // "You turned this down" is one drag of a slider away from coming back.
    // "This did not fit" is prose that has to be shortened. Merged into one
    // list, the advice is wrong half the time.
    const groups = dropGroups([
      { fragment: frag({ text: 'muted' }), reason: 'silenced' },
      { fragment: frag({ text: 'too long' }), reason: 'budget' },
      { fragment: frag({ text: 'also muted' }), reason: 'silenced' },
    ])

    expect(groups.silenced.map((d) => d.fragment.text)).toEqual(['muted', 'also muted'])
    expect(groups.budget.map((d) => d.fragment.text)).toEqual(['too long'])
  })

  it('reports no mood board reference as a casualty', () => {
    // A `moodboard_only` fragment did exactly what it was attached to do.
    // Listing it as dropped would send someone off to fix something that is not
    // broken — and would put its text on the one surface it must never reach.
    const groups = dropGroups([
      { fragment: frag({ text: 'her mother', sendable: false }), reason: 'silenced' },
      { fragment: frag({ text: null, assetId: 'a1' }), reason: 'budget' },
    ])
    expect(groups).toEqual({ silenced: [], budget: [] })
  })
})

describe('promptEmptiness', () => {
  it('separates "nothing was written" from "everything was cut"', () => {
    // The first is a world with no notes in it yet; the second is a world whose
    // notes all lost. One says go and write something, the other says go and
    // look at the drop report, and a single "empty" state would say neither.
    expect(promptEmptiness(compiled())).toBe('nothing-at-all')
    expect(
      promptEmptiness(
        compiled({ dropped: [{ fragment: frag({ text: 'muted' }), reason: 'silenced' }] }),
      ),
    ).toBe('all-cut')
    expect(promptEmptiness(compiled({ negative: 'blurry' }))).toBe('all-cut')
    expect(promptEmptiness(compiled({ prompt: 'ink wash' }))).toBeNull()
  })
})

describe('fragmentLabel', () => {
  it('names the layer, the source and the section, so tint is never the only carrier', () => {
    expect(
      fragmentLabel(frag({ layer: 'ancestry', sourceName: 'Ashkin', section: 'skin_texture' })),
    ).toBe('Ancestry · Ashkin · skin texture')
  })

  it('reads for the shot, which has a preset behind it and no node', () => {
    expect(
      fragmentLabel(
        frag({ layer: 'shot', nodeId: null, sourceName: 'Character sheet', section: 'framing' }),
      ),
    ).toBe('Shot · Character sheet · framing')
  })
})

describe('sectionLabel', () => {
  it('spells a key as prose without inventing a label the registry did not give', () => {
    expect(sectionLabel('full_ref')).toBe('full ref')
    expect(sectionLabel('silhouette')).toBe('silhouette')
  })
})
