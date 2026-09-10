import { beforeEach, expect, it, vi } from 'vitest'
import {
  narrativeTextCreate,
  narrativeTextDelete,
  narrativeTextDiagnostics,
  narrativeTextGet,
  narrativeTextSave,
  narrativeTexts,
  type TextAsset,
} from './narrativeText'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({})
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

const asset: TextAsset = {
  id: 'asset',
  kind: 'bark',
  name: 'Gate guard',
  trigger: { event: 'player_passes_gate' },
  repeat: 'shuffle',
  entries: [
    {
      id: 'entry',
      label: 'Move along',
      lines: [
        {
          id: 'line',
          speaker: { entity: 'guard' },
          variants: [{ id: 'variant', text: { revision: 'rev', body: 'Move along.' } }],
        },
      ],
    },
  ],
}

it('names every supporting text command and its arguments exactly', async () => {
  await narrativeTexts()
  expect(invoke).toHaveBeenLastCalledWith('narrative_texts', undefined)

  await narrativeTextGet('asset')
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_get', { assetId: 'asset' })

  await narrativeTextCreate('codex', 'Beacons', 'codex_opened')
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_create', {
    kind: 'codex',
    name: 'Beacons',
    event: 'codex_opened',
  })

  await narrativeTextDelete('asset')
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_delete', { assetId: 'asset' })
})

it('sends the document verbatim, with its snake_case source spelling intact', async () => {
  // The asset is the YAML in `narrative/texts/*.yaml`, so anything this side
  // renamed on the way across would be a field the backend silently refused.
  await narrativeTextSave(asset, { kind: 'stamp', stamp: { mtime_ms: 1, size: 2, hash: 'h' } })
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_save', {
    asset,
    expected: { kind: 'stamp', stamp: { mtime_ms: 1, size: 2, hash: 'h' } },
    slug: null,
  })
})

it('sends an explicit null rather than omitting the optional arguments', async () => {
  // Tauri distinguishes a missing argument from a null one, and the Rust
  // signatures take `Option`. Omitting them would be a deserialization failure
  // rather than the default the caller meant.
  await narrativeTextDiagnostics('asset')
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_diagnostics', {
    assetId: 'asset',
    asset: null,
  })
  await narrativeTextDiagnostics('asset', asset)
  expect(invoke).toHaveBeenLastCalledWith('narrative_text_diagnostics', {
    assetId: 'asset',
    asset,
  })
})
