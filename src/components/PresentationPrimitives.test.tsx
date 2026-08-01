import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Generation } from '../lib/api'
import { LazyAssetThumbnail } from './AssetMedia'
import {
  GenerationModelSeed,
  GenerationPresetModel,
  GenerationSubject,
  GenerationTimestamp,
} from './GenerationMetadata'

const h = vi.hoisted(() => ({ useAssetThumb: vi.fn() }))

vi.mock('../lib/queries', () => ({ useAssetThumb: h.useAssetThumb }))
vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}))

function generation(over: Partial<Generation> = {}): Generation {
  return {
    id: 'generation-1',
    nodeId: 'kael',
    createdAt: '2026-08-01T12:00:00Z',
    preset: 'portrait',
    viewType: null,
    userPrompt: '',
    compiledPrompt: 'ash-grey coat',
    negativePrompt: '',
    backend: 'comfyui',
    model: 'flux-dev',
    seed: 42,
    params: {},
    outputAssetIds: ['asset-1'],
    influenceSnapshot: { layers: [] },
    ...over,
  }
}

beforeEach(() => h.useAssetThumb.mockReset())

describe('LazyAssetThumbnail', () => {
  it('keeps missing, loading, error, and lazy-image presentation consumer-defined', () => {
    h.useAssetThumb.mockReturnValue({ data: undefined, isError: false })
    const view = render(
      <LazyAssetThumbnail
        assetId={null}
        alt="Receipt output"
        loadingLabel="Fetching receipt…"
        missingLabel="No output"
        errorLabel="Preview failed"
      />,
    )
    expect(screen.getByText('No output')).toBeInTheDocument()
    expect(h.useAssetThumb).toHaveBeenCalledWith(null)

    view.rerender(
      <LazyAssetThumbnail
        assetId="asset-1"
        alt="Receipt output"
        loadingLabel="Fetching receipt…"
        missingLabel="No output"
        errorLabel="Preview failed"
      />,
    )
    expect(screen.getByText('Fetching receipt…')).toBeInTheDocument()

    h.useAssetThumb.mockReturnValue({ data: undefined, isError: true })
    view.rerender(
      <LazyAssetThumbnail
        assetId="asset-1"
        alt="Receipt output"
        loadingLabel="Fetching receipt…"
        missingLabel="No output"
        errorLabel="Preview failed"
      />,
    )
    expect(screen.getByText('Preview failed')).toBeInTheDocument()

    h.useAssetThumb.mockReturnValue({ data: '/thumb/asset-1.webp', isError: false })
    view.rerender(
      <LazyAssetThumbnail
        assetId="asset-1"
        alt="Receipt output"
        loadingLabel="Fetching receipt…"
        missingLabel="No output"
        errorLabel="Preview failed"
      />,
    )
    expect(screen.getByRole('img', { name: 'Receipt output' })).toHaveAttribute(
      'src',
      'asset:///thumb/asset-1.webp',
    )
    expect(screen.getByRole('img', { name: 'Receipt output' })).toHaveAttribute('loading', 'lazy')
  })
})

describe('generation metadata', () => {
  it('renders the same entity receipt identity in each supported arrangement', () => {
    const receipt = generation()
    render(
      <div>
        <span data-testid="subject">
          <GenerationSubject generation={receipt} fallback="Kael" />
        </span>
        <span data-testid="model-seed">
          <GenerationModelSeed generation={receipt} includeBackend />
        </span>
        <span data-testid="preset-model">
          <GenerationPresetModel generation={receipt} />
        </span>
        <span data-testid="timestamp">
          <GenerationTimestamp generation={receipt} />
        </span>
      </div>,
    )

    expect(screen.getByTestId('subject')).toHaveTextContent(/^Kael$/)
    expect(screen.getByTestId('model-seed')).toHaveTextContent(/^comfyui \/ flux-dev · seed 42$/)
    expect(screen.getByTestId('preset-model')).toHaveTextContent(/^portrait · flux-dev$/)
    expect(screen.getByTestId('timestamp')).toHaveTextContent(
      new Date(receipt.createdAt).toLocaleString(),
    )
  })

  it('preserves multi-entity scene authorship', () => {
    const receipt = generation({
      params: {
        sceneComposition: {
          version: 1,
          subjectIds: ['kael', 'mira'],
          subjectNames: ['Kael', 'Mira'],
        },
      },
    })
    render(
      <div>
        <span data-testid="subject">
          <GenerationSubject generation={receipt} fallback="Fallback" />
        </span>
        <span data-testid="preset-model">
          <GenerationPresetModel generation={receipt} />
        </span>
      </div>,
    )

    expect(screen.getByTestId('subject')).toHaveTextContent(/^Scene · Kael \+ Mira$/)
    expect(screen.getByTestId('preset-model')).toHaveTextContent(
      /^Multi-entity · portrait · flux-dev$/,
    )
  })
})
