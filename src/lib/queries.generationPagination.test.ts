import type { InfiniteData } from '@tanstack/react-query'
import { describe, expect, it } from 'vitest'
import type { GenerationPage, GenerationSummary } from './api'
import { GENERATION_PAGE_SIZE, prependGeneration } from './queries'

function summary(id: string): GenerationSummary {
  return {
    id,
    nodeId: 'kael',
    createdAt: '2026-08-02T12:00:00Z',
    preset: 'portrait',
    viewType: null,
    backend: 'comfyui',
    model: 'flux-dev',
    seed: 42,
    promptExcerpt: id,
    firstAssetId: null,
    outputCount: 0,
    seedSource: null,
    usedLockedSeed: null,
    sceneSubjectNames: [],
    thumbnailPath: null,
  }
}

function page(start: number, nextOffset: number | null): GenerationPage {
  return {
    items: Array.from({ length: GENERATION_PAGE_SIZE }, (_, index) =>
      summary(`old-${start + index}`),
    ),
    total: 180,
    nextOffset,
  }
}

describe('targeted generation cache insertion', () => {
  it('carries displaced rows across loaded pages without skipping the next backend offset', () => {
    const cached: InfiniteData<GenerationPage, number> = {
      pages: [page(0, 60), page(60, 120)],
      pageParams: [0, 60],
    }
    const updated = prependGeneration(cached, summary('new'))!
    const loaded = updated.pages.flatMap((value) => value.items.map((item) => item.id))

    expect(loaded).toEqual(['new', ...Array.from({ length: 119 }, (_, index) => `old-${index}`)])
    expect(new Set(loaded).size).toBe(loaded.length)
    expect(updated.pages[1]?.nextOffset).toBe(120)

    const nextBackendPage = Array.from({ length: 60 }, (_, index) => `old-${119 + index}`)
    expect([...loaded, ...nextBackendPage]).toEqual([
      'new',
      ...Array.from({ length: 179 }, (_, index) => `old-${index}`),
    ])
  })
})
