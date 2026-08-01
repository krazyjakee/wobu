import { describe, expect, it } from 'vitest'
import type { CompiledPrompt, Generation, InfluenceStack } from './api'
import { generationDrift } from './generationDiff'

const generation = {
  id: 'old',
  nodeId: 'kael',
  createdAt: '2026-08-01T00:00:00Z',
  preset: 'portrait',
  viewType: null,
  userPrompt: '',
  compiledPrompt: 'old prompt',
  negativePrompt: '',
  backend: 'comfyui',
  model: 'local',
  seed: 4,
  params: {},
  outputAssetIds: [],
  influenceSnapshot: {
    layers: [
      {
        layer: 'subject',
        nodeId: 'kael',
        nodeName: 'Kael',
        weight: 0.7,
        muted: false,
        fragments: [
          {
            section: 'appearance',
            text: 'old coat',
            assetId: null,
            weight: 0.7,
            target: 'prompt',
            dropped: false,
          },
        ],
      },
    ],
  },
} satisfies Generation

const stack = {
  subjectId: 'kael',
  preset: {
    id: 'portrait',
    label: 'Portrait',
    kinds: ['character'],
    defaultFor: [],
    priorities: [],
    framing: '',
    aspect: '3:4',
    images: 1,
    views: [],
    imageConstraints: null,
  },
  layers: [
    {
      layer: 'subject',
      nodeId: 'kael',
      name: 'Kael',
      kind: 'character',
      reached: 'subject',
      distance: 0,
      weight: 1,
      slider: 1,
      fragments: [
        {
          layer: 'subject',
          nodeId: 'kael',
          sourceName: 'Kael',
          section: 'appearance',
          text: 'new coat',
          assetId: null,
          weight: 1,
          target: 'prompt',
          sendable: true,
        },
      ],
    },
  ],
} satisfies InfluenceStack

const prompt = {
  subjectId: 'kael',
  preset: stack.preset,
  prompt: 'new prompt',
  negative: '',
  spans: [],
  dropped: [],
  overflow: null,
} satisfies CompiledPrompt

describe('generation drift', () => {
  it('reports prompt, weight, and fragment changes against the immutable snapshot', () => {
    const drift = generationDrift(generation, stack, prompt)
    expect(drift?.promptChanged).toBe(true)
    expect(drift?.layers[0]?.status).toBe('changed')
    expect(drift?.layers[0]?.changes).toEqual(['weight', 'fragments'])
  })

  it('does not call provider drops or an incomparable legacy negative prompt world drift', () => {
    const dropped = {
      ...generation,
      negativePrompt: '',
      influenceSnapshot: {
        layers: [
          {
            ...generation.influenceSnapshot.layers[0]!,
            weight: 1,
            fragments: [
              {
                ...generation.influenceSnapshot.layers[0]!.fragments[0]!,
                text: 'new coat',
                weight: 1,
                dropped: true,
              },
            ],
          },
        ],
      },
    }
    const current = { ...prompt, prompt: 'old prompt', negative: 'new negative constraint' }
    const drift = generationDrift(dropped, stack, current)
    expect(drift?.layers[0]?.status).toBe('unchanged')
    expect(drift?.negativeComparable).toBe(false)
    expect(drift?.negativeChanged).toBe(false)
  })
})
