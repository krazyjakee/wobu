import { useEffect, useMemo, useRef, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import * as api from '../../lib/api'
import type { Asset, AssetLink, AssetRole, WobuNode } from '../../lib/api'
import type { useAutosaveNode } from '../../hooks/useAutosaveNode'
import { useAssetThumb, useAssets, useLinkAsset, useSetCoverAsset } from '../../lib/queries'

type Autosave = ReturnType<typeof useAutosaveNode>
type ImportState = 'queued' | 'importing' | 'linking' | 'done' | 'failed'
type ImportItem = {
  id: number
  name: string
  state: ImportState
  error?: string
  deduped?: boolean
}
type ImportInput =
  { source: 'path'; name: string; path: string } | { source: 'file'; name: string; file: File }

const DEFAULT_ROLE: AssetRole = 'full_ref'
const TILE_MIN = 196
const TILE_HEIGHT = 302
const GAP = 12
const OVERSCAN = 2

export function ReferencesPane({
  node,
  readOnly,
  autosave,
}: {
  node: WobuNode
  readOnly: boolean
  autosave: Autosave
}) {
  const assets = useAssets(true)
  const linkAsset = useLinkAsset()
  const setCover = useSetCoverAsset()
  const [links, setLinks] = useState(node.assetLinks)
  const [imports, setImports] = useState<ImportItem[]>([])
  const [importedAssets, setImportedAssets] = useState<Record<string, Asset>>({})
  const [draggingFiles, setDraggingFiles] = useState(false)
  const importId = useRef(0)
  const importChain = useRef<Promise<void>>(Promise.resolve())
  const saveStatus = useRef(autosave.status)

  useEffect(() => setLinks(node.assetLinks), [node.assetLinks])
  useEffect(() => {
    if (readOnly) setDraggingFiles(false)
  }, [readOnly])
  useEffect(() => {
    saveStatus.current = autosave.status
  }, [autosave.status])

  const assetIndex = useMemo(() => {
    const index = new Map((assets.data ?? []).map((asset) => [asset.id, asset]))
    for (const asset of Object.values(importedAssets)) index.set(asset.id, asset)
    return index
  }, [assets.data, importedAssets])

  const updateLinks = (next: AssetLink[]) => {
    setLinks(next)
    autosave.queue({ assetLinks: next })
  }

  const updateImport = (id: number, patch: Partial<ImportItem>) => {
    setImports((current) => current.map((item) => (item.id === id ? { ...item, ...patch } : item)))
  }

  const importInputs = async (inputs: ImportInput[]) => {
    if (readOnly || inputs.length === 0) return
    const batch = inputs.map((input) => ({
      input,
      item: { id: ++importId.current, name: input.name, state: 'queued' as const },
    }))
    setImports((current) => [...batch.map(({ item }) => item), ...current])

    importChain.current = importChain.current
      .catch(() => undefined)
      .then(async () => {
        try {
          await settleLinkAutosave()
        } catch (error) {
          for (const { item } of batch) {
            updateImport(item.id, {
              state: 'failed',
              error: `Could not attach: ${api.errorMessage(error)}`,
            })
          }
          return
        }
        // Sequential attachment is intentional. Each link is a guarded edit to
        // the same Markdown file; concurrent saves would race one another and
        // turn a successful forty-file drop into conflicts created by ourselves.
        for (const { input, item } of batch) {
          let imported = false
          try {
            updateImport(item.id, { state: 'importing' })
            const result =
              input.source === 'path'
                ? await api.assetImport(input.path, 'reference')
                : await api.assetImportBytes(
                    new Uint8Array(await input.file.arrayBuffer()),
                    'reference',
                  )
            imported = true
            setImportedAssets((current) => ({ ...current, [result.asset.id]: result.asset }))
            updateImport(item.id, { state: 'linking', deduped: result.deduped })
            const saved = await linkAsset.mutateAsync({
              nodeId: node.id,
              assetId: result.asset.id,
              role: DEFAULT_ROLE,
            })
            setLinks(saved.assetLinks)
            updateImport(item.id, { state: 'done' })
          } catch (error) {
            updateImport(item.id, {
              state: 'failed',
              error: `${imported ? 'Imported, but could not attach' : 'Import failed'}: ${api.errorMessage(error)}`,
            })
          }
        }
      })
    await importChain.current
  }

  const settleLinkAutosave = async () => {
    if (saveStatus.current === 'dirty') autosave.flush()
    while (saveStatus.current === 'dirty' || saveStatus.current === 'saving') {
      await new Promise<void>((resolve) => window.setTimeout(resolve, 25))
    }
    if (saveStatus.current === 'error' || saveStatus.current === 'held') {
      throw new Error('The pending reference edit must save before more images can be attached.')
    }
  }

  useEffect(() => {
    if (!api.isTauri() || readOnly) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'enter' || event.payload.type === 'over') {
          setDraggingFiles(true)
        } else if (event.payload.type === 'leave') {
          setDraggingFiles(false)
        } else if (event.payload.type === 'drop') {
          setDraggingFiles(false)
          void importInputs(
            event.payload.paths.map((path) => ({
              source: 'path',
              path,
              name: fileName(path),
            })),
          )
        }
      })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* HTML drop remains available in browser/dev mode */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [node.id, readOnly])

  const chooseFiles = async () => {
    if (readOnly) return
    const selected = await open({
      multiple: true,
      directory: false,
      filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }],
    })
    const paths = selected ? (Array.isArray(selected) ? selected : [selected]) : []
    await importInputs(paths.map((path) => ({ source: 'path', path, name: fileName(path) })))
  }

  const receiveFiles = (files: FileList | File[]) => {
    void importInputs(
      Array.from(files).map((file) => ({
        source: 'file',
        name: file.name || 'Pasted image',
        file,
      })),
    )
  }

  const importActive = imports.some(
    (item) => item.state === 'queued' || item.state === 'importing' || item.state === 'linking',
  )

  return (
    <section
      className={`references${!readOnly && draggingFiles ? ' is-file-drag' : ''}`}
      aria-label={`References for ${node.name}`}
      tabIndex={0}
      onDragEnter={(event) => {
        if (readOnly) return
        if (event.dataTransfer.types.includes('Files')) {
          event.preventDefault()
          setDraggingFiles(true)
        }
      }}
      onDragOver={(event) => {
        if (readOnly) return
        if (event.dataTransfer.types.includes('Files')) event.preventDefault()
      }}
      onDragLeave={(event) => {
        if (readOnly) return
        if (!event.currentTarget.contains(event.relatedTarget as Node | null))
          setDraggingFiles(false)
      }}
      onDrop={(event) => {
        if (readOnly) return
        if (!event.dataTransfer.files.length) return
        event.preventDefault()
        setDraggingFiles(false)
        receiveFiles(event.dataTransfer.files)
      }}
      onPaste={(event) => {
        if (readOnly) return
        const files = Array.from(event.clipboardData.files).filter((file) =>
          file.type.startsWith('image/'),
        )
        if (files.length) {
          event.preventDefault()
          receiveFiles(files)
        }
      }}
    >
      <header className="references-head">
        <div>
          <h2>Reference board</h2>
          <p>Drop or paste images. Roles decide how each one influences generation.</p>
        </div>
        <button
          className="btn"
          type="button"
          disabled={readOnly || importActive}
          onClick={() => void chooseFiles()}
        >
          Add images…
        </button>
      </header>

      {imports.length > 0 && <ImportReport items={imports} onClear={() => setImports([])} />}

      {assets.isError && (
        <p className="references-error">
          Could not read the asset library: {api.errorMessage(assets.error)}
        </p>
      )}

      <VirtualReferenceGrid
        links={links}
        assets={assetIndex}
        coverAssetId={node.coverAssetId}
        readOnly={readOnly || importActive}
        onChange={updateLinks}
        onCover={(assetId) => setCover.mutate({ nodeId: node.id, assetId })}
      />

      {!readOnly && draggingFiles && (
        <div className="references-drop">Drop images to import and attach</div>
      )}
    </section>
  )
}

function ImportReport({ items, onClear }: { items: ImportItem[]; onClear: () => void }) {
  const active = items.filter((item) => item.state !== 'done' && item.state !== 'failed').length
  const failed = items.filter((item) => item.state === 'failed').length
  const done = items.filter((item) => item.state === 'done').length
  return (
    <details className="reference-imports" open={active > 0 || failed > 0}>
      <summary>
        Importing {items.length} {items.length === 1 ? 'file' : 'files'} · {done} done · {failed}{' '}
        failed
      </summary>
      <div className="reference-import-list">
        {items.map((item) => (
          <div className={`reference-import is-${item.state}`} key={item.id}>
            <span>{item.name}</span>
            <b>{importLabel(item)}</b>
            {item.error && <small>{item.error}</small>}
          </div>
        ))}
      </div>
      {active === 0 && (
        <button type="button" onClick={onClear}>
          Clear report
        </button>
      )}
    </details>
  )
}

function VirtualReferenceGrid({
  links,
  assets,
  coverAssetId,
  readOnly,
  onChange,
  onCover,
}: {
  links: AssetLink[]
  assets: Map<string, Asset>
  coverAssetId: string | null
  readOnly: boolean
  onChange: (links: AssetLink[]) => void
  onCover: (assetId: string | null) => void
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 800, height: 600 })
  const [scrollTop, setScrollTop] = useState(0)
  const [dragged, setDragged] = useState<number | null>(null)

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const columns = Math.max(1, Math.floor((size.width + GAP) / (TILE_MIN + GAP)))
  const tileWidth = (size.width - GAP * (columns - 1)) / columns
  const rows = Math.ceil(links.length / columns)
  const startRow = Math.max(0, Math.floor(scrollTop / TILE_HEIGHT) - OVERSCAN)
  const endRow = Math.min(rows, Math.ceil((scrollTop + size.height) / TILE_HEIGHT) + OVERSCAN)
  const start = startRow * columns
  const end = Math.min(links.length, endRow * columns)

  const move = (from: number, to: number) => {
    if (from === to || from < 0 || to < 0 || from >= links.length || to >= links.length) return
    const next = [...links]
    const [item] = next.splice(from, 1)
    if (!item) return
    next.splice(to, 0, item)
    onChange(next)
  }

  if (links.length === 0) {
    return (
      <div className="references-empty">
        <h3>No references yet</h3>
        <p>
          Drop images here or paste from the clipboard to build this entity&apos;s visual canon.
        </p>
      </div>
    )
  }

  return (
    <div
      className="reference-grid-viewport"
      ref={viewport}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div className="reference-grid" style={{ height: rows * TILE_HEIGHT }}>
        {links.slice(start, end).map((link, offset) => {
          const index = start + offset
          const row = Math.floor(index / columns)
          const column = index % columns
          return (
            <ReferenceTile
              key={`${link.assetId}-${link.role}`}
              link={link}
              asset={assets.get(link.assetId)}
              index={index}
              count={links.length}
              width={tileWidth}
              top={row * TILE_HEIGHT}
              left={column * (tileWidth + GAP)}
              isCover={coverAssetId === link.assetId}
              readOnly={readOnly}
              usedRoles={links
                .filter(
                  (candidate, candidateIndex) =>
                    candidate.assetId === link.assetId && candidateIndex !== index,
                )
                .map((candidate) => candidate.role)}
              onUpdate={(patch) => {
                const next = [...links]
                next[index] = { ...link, ...patch }
                onChange(next)
              }}
              onRemove={() => onChange(links.filter((_, linkIndex) => linkIndex !== index))}
              onCover={() => onCover(isCover(coverAssetId, link.assetId) ? null : link.assetId)}
              onMove={(to) => move(index, to)}
              onDragStart={() => setDragged(index)}
              onDrop={() => {
                if (dragged !== null) move(dragged, index)
                setDragged(null)
              }}
            />
          )
        })}
      </div>
    </div>
  )
}

function ReferenceTile({
  link,
  asset,
  index,
  count,
  width,
  top,
  left,
  isCover,
  readOnly,
  usedRoles,
  onUpdate,
  onRemove,
  onCover,
  onMove,
  onDragStart,
  onDrop,
}: {
  link: AssetLink
  asset: Asset | undefined
  index: number
  count: number
  width: number
  top: number
  left: number
  isCover: boolean
  readOnly: boolean
  usedRoles: AssetRole[]
  onUpdate: (patch: Partial<AssetLink>) => void
  onRemove: () => void
  onCover: () => void
  onMove: (index: number) => void
  onDragStart: () => void
  onDrop: () => void
}) {
  const thumb = useAssetThumb(link.assetId)
  return (
    <article
      className={`reference-tile${link.enabled ? '' : ' is-muted'}${isCover ? ' is-cover' : ''}`}
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
      draggable={!readOnly}
      onDragStart={(event) => {
        event.dataTransfer.setData('application/x-wobu-reference', String(index))
        event.dataTransfer.effectAllowed = 'move'
        onDragStart()
      }}
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes('application/x-wobu-reference')) {
          event.preventDefault()
          event.dataTransfer.dropEffect = 'move'
        }
      }}
      onDrop={(event) => {
        if (!event.dataTransfer.types.includes('application/x-wobu-reference')) return
        event.preventDefault()
        event.stopPropagation()
        onDrop()
      }}
    >
      <div className="reference-image">
        {thumb.data ? (
          <img src={convertFileSrc(thumb.data)} alt={`${roleLabel(link.role)} reference`} />
        ) : (
          <span>{thumb.isError ? 'Preview failed' : 'Loading preview…'}</span>
        )}
        {isCover && <b>Cover</b>}
      </div>
      <div className="reference-controls">
        <select
          aria-label={`Role for reference ${index + 1}`}
          value={link.role}
          disabled={readOnly}
          onChange={(event) => onUpdate({ role: event.target.value as AssetRole })}
        >
          {api.ASSET_ROLES.map((role) => (
            <option key={role} value={role} disabled={usedRoles.includes(role)}>
              {roleLabel(role)}
            </option>
          ))}
        </select>
        <label>
          <span>Weight</span>
          <input
            aria-label={`Weight for reference ${index + 1}`}
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={link.weight}
            disabled={readOnly}
            onChange={(event) => onUpdate({ weight: Number(event.target.value) })}
          />
          <output>{link.weight.toFixed(2)}</output>
        </label>
        <div className="reference-actions">
          <button
            type="button"
            disabled={readOnly || index === 0}
            onClick={() => onMove(index - 1)}
          >
            ←
          </button>
          <button
            type="button"
            disabled={readOnly || index === count - 1}
            onClick={() => onMove(index + 1)}
          >
            →
          </button>
          <button
            type="button"
            disabled={readOnly}
            onClick={() => onUpdate({ enabled: !link.enabled })}
          >
            {link.enabled ? 'Mute' : 'Unmute'}
          </button>
          <button type="button" disabled={readOnly} onClick={onCover}>
            {isCover ? 'Clear cover' : 'Set cover'}
          </button>
          <button type="button" disabled={readOnly} onClick={onRemove}>
            Remove
          </button>
        </div>
      </div>
      <small>
        {asset ? `${asset.width}×${asset.height} · ${formatBytes(asset.bytes)}` : link.assetId}
      </small>
    </article>
  )
}

function importLabel(item: ImportItem): string {
  if (item.state === 'queued') return 'Waiting'
  if (item.state === 'importing') return 'Importing…'
  if (item.state === 'linking') return 'Attaching…'
  if (item.state === 'failed') return 'Failed'
  return item.deduped ? 'Already present · attached' : 'Imported · attached'
}

function roleLabel(role: AssetRole): string {
  return role === 'full_ref' ? 'Full reference' : `${role.charAt(0).toUpperCase()}${role.slice(1)}`
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`
  if (bytes < 1_048_576) return `${Math.round(bytes / 1_024)} KB`
  return `${(bytes / 1_048_576).toFixed(1)} MB`
}

function isCover(coverAssetId: string | null, assetId: string): boolean {
  return coverAssetId === assetId
}
