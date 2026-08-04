import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { kindDef, kindIndex } from '../test/fixtures'
import { StyleTransferSheet } from './StyleTransferSheet'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))

const kinds = kindIndex([
  kindDef('style_guide', { label: 'Art Style', singleton: true, nests: false }),
])

function preview(missingAssetCount = 0) {
  return {
    version: 1,
    sourceProjectId: 'source',
    sourceProjectName: 'House Style',
    defaultRootId: 'style',
    pinnedLoras: [],
    loraNote: 'ComfyUI LoRAs are installed on this computer and are not copied.',
    candidates: [
      {
        rootId: 'style',
        kind: 'style_guide',
        name: 'Ink Wash',
        nodeCount: 1,
        referenceCount: 2,
        externalLinkCount: 0,
        missingAssetCount,
        replacesSingleton: true,
      },
    ],
  }
}

function open(onClose = () => {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <StyleTransferSheet
        sourcePath="/source.wobu"
        kinds={kinds}
        onClose={onClose}
        onImported={() => {}}
      />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.invoke.mockReset()
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('StyleTransferSheet', () => {
  it('starts on the safe Close action and dismisses with Escape', async () => {
    h.invoke.mockResolvedValue(preview())
    const onClose = vi.fn()
    open(onClose)

    const dialog = await screen.findByRole('dialog', { name: 'Import from another project' })
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleDescription(/Reading the source project|Choose one entity/)
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('blocks apply when a referenced source blob is missing', async () => {
    h.invoke.mockResolvedValue(preview(1))
    open()
    expect(await screen.findByText(/Restore them before importing/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Replace and import' })).toBeDisabled()
    expect(h.invoke).toHaveBeenCalledTimes(1)
  })

  it('keeps a partial guarded-write report visible with pending recovery details', async () => {
    h.invoke.mockImplementation((command: string) => {
      if (command === 'style_transfer_preview') return Promise.resolve(preview())
      if (command === 'style_transfer_apply') {
        return Promise.resolve({
          completed: false,
          rootId: 'style',
          importedRootId: 'destination-style',
          plannedNodeCount: 1,
          appliedNodeIds: [],
          pendingNodeIds: ['destination-style'],
          referenceCount: 2,
          dedupedReferenceCount: 0,
          droppedExternalLinkCount: 0,
          replacedSingleton: true,
          conflictPaths: ['nodes/style-guide/style.conflict-nadia.md'],
          failure: 'A destination node changed during transfer.',
        })
      }
      return Promise.resolve(null)
    })
    open()
    fireEvent.click(await screen.findByRole('button', { name: 'Replace and import' }))
    expect(await screen.findByRole('status')).toHaveTextContent(
      'The import stopped after 0 of 1 entities',
    )
    expect(screen.getByRole('status')).toHaveTextContent('1 entity was not brought across')
    expect(screen.getByRole('status')).toHaveTextContent('style.conflict-nadia.md')
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('style_transfer_apply', expect.anything()),
    )
  })
})
