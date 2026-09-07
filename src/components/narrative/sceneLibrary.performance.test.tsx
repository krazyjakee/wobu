import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { GenerationPolicy } from '../../lib/api'
import { NarrativeLibrary } from './NarrativeLibrary'
import { DEFAULT_LIBRARY_VIEW, findScenes, type LibraryRow } from './sceneLibraryModel'

const SCENE_COUNT = 1000
const SLOTS_PER_SCENE = 50
const PAGE_SIZE = 25
// Regression ceilings, not frame-time or native responsiveness claims. They
// leave headroom for shared CI machines while catching accidental quadratic work.
const SEARCH_BUDGET_MS = 1500
const MOUNT_BUDGET_MS = 5000
const PAGE_BUDGET_MS = 2000
const POLICIES: GenerationPolicy[] = ['locked', 'edited', 'generated']

/** Fixed identities and varied prose; no IPC, filesystem reads or index build. */
const CORPUS: LibraryRow[] = Array.from({ length: SCENE_COUNT }, (_, sceneIndex) => {
  const id = `scene-${String(sceneIndex).padStart(4, '0')}`
  const name = `Scene ${String(sceneIndex).padStart(4, '0')}`
  const participant = `character-${sceneIndex % 10}`
  return {
    summary: { id, name, slug: id, rel: `narrative/scenes/${id}.yaml` },
    scene: {
      id,
      name,
      summary: `The expedition visits region ${sceneIndex % 17}.`,
      participants: [{ entity: participant }],
      beats: Array.from({ length: 5 }, (_, beatIndex) => ({
        id: `${id}-beat-${beatIndex}`,
        title: `Expedition stage ${beatIndex}`,
        intents: [
          {
            subject: 'player' as const,
            intent: `Negotiate passage through sector ${sceneIndex}-${beatIndex}.`,
          },
        ],
        dialogue: Array.from({ length: 10 }, (_, lineIndex) => {
          const slotIndex = beatIndex * 10 + lineIndex
          const slotId = `${id}-slot-${slotIndex}`
          return {
            id: slotId,
            speaker: { entity: participant },
            policy: POLICIES[sceneIndex % 3]!,
            variants: [
              {
                id: `${slotId}-variant`,
                text: {
                  revision: `${slotId}-revision`,
                  body:
                    sceneIndex === 777 && slotIndex === 49
                      ? 'The silver beacon fell silent beyond the eastern ridge.'
                      : `Our route crosses district ${sceneIndex % 17}, checkpoint ${slotIndex}. Courier ${sceneIndex} expects the tide at hour ${(sceneIndex + slotIndex) % 24}.`,
                  provenance: 'human' as const,
                  lifecycle: {
                    policy: POLICIES[sceneIndex % 3]!,
                    review: sceneIndex % 2 ? ('approved' as const) : ('draft' as const),
                    freshness:
                      sceneIndex % 5 === 2 ? ('out_of_date' as const) : ('current' as const),
                  },
                },
              },
            ],
          }
        }),
      })),
    },
  }
})

function measure<T>(operation: () => T): { value: T; ms: number } {
  const start = performance.now()
  const value = operation()
  return { value, ms: performance.now() - start }
}

beforeEach(() => localStorage.clear())

describe('Scene library: 1,000 loaded scenes / 50,000 dialogue slots', () => {
  it('finds a remembered line and combines independent lifecycle facets within the loaded-memory budget', () => {
    expect(CORPUS).toHaveLength(SCENE_COUNT)
    expect(
      CORPUS.reduce(
        (count, row) =>
          count + (row.scene?.beats?.reduce((n, beat) => n + (beat.dialogue?.length ?? 0), 0) ?? 0),
        0,
      ),
    ).toBe(SCENE_COUNT * SLOTS_PER_SCENE)
    const lineSamples = Array.from({ length: 3 }, () =>
      measure(() =>
        findScenes(CORPUS, {
          ...DEFAULT_LIBRARY_VIEW,
          query: 'silver beacon',
        }),
      ),
    )
    for (const sample of lineSamples) {
      expect(sample.value).toHaveLength(1)
      expect(sample.value[0]?.matches).toEqual([
        expect.objectContaining({
          sceneId: 'scene-0777',
          beatId: 'scene-0777-beat-4',
          lineId: 'scene-0777-slot-49',
          variantId: 'scene-0777-slot-49-variant',
        }),
      ])
      expect(sample.ms).toBeLessThan(SEARCH_BUDGET_MS)
    }
    const facets = measure(() =>
      findScenes(CORPUS, {
        ...DEFAULT_LIBRARY_VIEW,
        query: 'route',
        participant: 'character-7',
        policy: 'locked',
        review: 'approved',
        freshness: 'out_of_date',
      }),
    )
    const expectedIds = Array.from({ length: SCENE_COUNT }, (_, index) => index)
      .filter((index) => index % 10 === 7 && index % 3 === 0)
      .map((index) => `scene-${String(index).padStart(4, '0')}`)
    expect(facets.value.map((row) => row.summary.id)).toEqual(expectedIds)
    expect(facets.value).toHaveLength(33)
    expect(facets.ms).toBeLessThan(SEARCH_BUDGET_MS)
    console.info('Loaded-memory scene search (ms)', {
      rememberedLine: lineSamples.map((sample) => Number(sample.ms.toFixed(1))),
      combinedFacets: Number(facets.ms.toFixed(1)),
      budgetPerOperation: SEARCH_BUDGET_MS,
    })
  })

  it('mounts exactly 25 result rows and retains paging through an open/return handoff', () => {
    const onOpen = vi.fn()
    const mountLibrary = () =>
      render(
        <NarrativeLibrary
          projectKey="/performance-world"
          rows={CORPUS}
          loading={false}
          readOnly={false}
          navCollapsed={false}
          nameOf={(id) => id}
          onOpen={onOpen}
          onCreateScene={vi.fn()}
        />,
      )
    const mount = measure(mountLibrary)
    expect(mount.ms).toBeLessThan(MOUNT_BUDGET_MS)
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(PAGE_SIZE + 1)
    expect(screen.getByRole('status')).toHaveTextContent('1000 matching of 1000 scenes')
    const page = measure(() => fireEvent.click(screen.getByRole('button', { name: 'Next page' })))
    expect(page.ms).toBeLessThan(PAGE_BUDGET_MS)
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(PAGE_SIZE + 1)
    fireEvent.click(screen.getByRole('button', { name: 'Open Scene 0025 in Script' }))
    expect(onOpen).toHaveBeenCalledWith(
      expect.objectContaining({ sceneId: 'scene-0025' }),
      'script',
      undefined,
    )
    mount.value.unmount()
    const returned = measure(mountLibrary)
    expect(returned.ms).toBeLessThan(MOUNT_BUDGET_MS)
    expect(screen.getByText('Page 2 of 40')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Open Scene 0025 in Script' })).toBeInTheDocument()
    expect(within(screen.getByRole('table')).getAllByRole('row')).toHaveLength(PAGE_SIZE + 1)
    console.info('Scene library jsdom render (ms)', {
      mount: Number(mount.ms.toFixed(1)),
      page: Number(page.ms.toFixed(1)),
      return: Number(returned.ms.toFixed(1)),
      mountBudget: MOUNT_BUDGET_MS,
      pageBudget: PAGE_BUDGET_MS,
      mountedSceneRows: PAGE_SIZE,
    })
  })
})
