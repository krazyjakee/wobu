import { useEffect, useMemo, useRef, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Asset, AssetKind, AssetRole, AssetUsage, NodeSummary } from '../lib/api'
import type { KindIndex } from '../lib/kinds'
import { labelFor } from '../lib/kinds'
import {
  useAssets,
  useAssetThumb,
  useAssetUsages,
  useDeleteAsset,
  useLinkAsset,
} from '../lib/queries'
import { useUI } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'

const KINDS: AssetKind[] = ['reference', 'generated', 'upload']
const TILE_MIN = 174
const TILE_HEIGHT = 216
const GAP = 12
const OVERSCAN = 2

export function AssetsMode({
  nodes,
  kinds,
  readOnly,
  onJump,
}: {
  nodes: NodeSummary[]
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
}) {
  const setMode = useUI((state) => state.setMode)
  const assets = useAssets(true)
  const usages = useAssetUsages(true)
  const deleteAsset = useDeleteAsset()
  const [kind, setKind] = useState<AssetKind | 'all'>('all')
  const [role, setRole] = useState<AssetRole | 'all'>('all')
  const [nodeId, setNodeId] = useState('all')
  const [tag, setTag] = useState('all')
  const [orphansOnly, setOrphansOnly] = useState(false)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<Asset | null>(null)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const usageKnown = usages.isSuccess

  const usageByAsset = useMemo(() => {
    const index = new Map<string, AssetUsage[]>()
    for (const usage of usages.data ?? []) {
      const current = index.get(usage.assetId)
      if (current) current.push(usage)
      else index.set(usage.assetId, [usage])
    }
    return index
  }, [usages.data])
  const tags = useMemo(
    () =>
      [...new Set((usages.data ?? []).flatMap((usage) => usage.nodeTags))].sort((a, b) =>
        a.localeCompare(b, undefined, { sensitivity: 'base' }),
      ),
    [usages.data],
  )
  const orphanCount = usageKnown
    ? (assets.data ?? []).filter((asset) => (usageByAsset.get(asset.id)?.length ?? 0) === 0).length
    : 0
  const filtered = useMemo(
    () =>
      (assets.data ?? []).filter((asset) => {
        const usedBy = usageByAsset.get(asset.id) ?? []
        if (kind !== 'all' && asset.kind !== kind) return false
        if (orphansOnly && (!usageKnown || usedBy.length > 0)) return false
        if (role !== 'all' && !usedBy.some((usage) => usage.roles.some((r) => r.role === role))) {
          return false
        }
        if (nodeId !== 'all' && !usedBy.some((usage) => usage.nodeId === nodeId)) return false
        if (tag !== 'all' && !usedBy.some((usage) => usage.nodeTags.includes(tag))) return false
        return true
      }),
    [assets.data, kind, nodeId, orphansOnly, role, tag, usageByAsset, usageKnown],
  )
  const selected = (assets.data ?? []).find((asset) => asset.id === selectedId) ?? null

  useEffect(() => {
    if (selectedId && !filtered.some((asset) => asset.id === selectedId)) setSelectedId(null)
  }, [filtered, selectedId])

  async function removeConfirmed() {
    if (!confirmDelete || readOnly) return
    setDeleteError(null)
    try {
      await deleteAsset.mutateAsync(confirmDelete.id)
      if (selectedId === confirmDelete.id) setSelectedId(null)
      setConfirmDelete(null)
    } catch (error) {
      setDeleteError(api.errorMessage(error))
      setConfirmDelete(null)
    }
  }

  return (
    <main className="assets-mode" aria-label="Asset library">
      <header className="assets-mode-head">
        <div>
          <h2>Assets</h2>
          <p>
            {assets.data?.length ?? 0} images · {orphanCount} orphaned · thumbnails stay lazy
          </p>
        </div>
        <button className="btn" type="button" onClick={() => setMode('library')}>
          Back to Library
        </button>
      </header>

      <div className="asset-filters" aria-label="Asset filters">
        <label>
          <span>Kind</span>
          <select
            aria-label="Filter assets by kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as AssetKind | 'all')}
          >
            <option value="all">All kinds</option>
            {KINDS.map((value) => (
              <option key={value} value={value}>
                {kindLabel(value)}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Role</span>
          <select
            aria-label="Filter assets by role"
            value={role}
            disabled={!usageKnown}
            onChange={(event) => setRole(event.target.value as AssetRole | 'all')}
          >
            <option value="all">All roles</option>
            {api.ASSET_ROLES.map((value) => (
              <option key={value} value={value}>
                {roleLabel(value)}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Node</span>
          <select
            aria-label="Filter assets by node"
            value={nodeId}
            disabled={!usageKnown}
            onChange={(event) => setNodeId(event.target.value)}
          >
            <option value="all">All nodes</option>
            {[...nodes]
              .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
              .map((node) => (
                <option key={node.id} value={node.id}>
                  {node.name}
                </option>
              ))}
          </select>
        </label>
        <label>
          <span>Linked-node tag</span>
          <select
            aria-label="Filter assets by linked node tag"
            value={tag}
            disabled={!usageKnown}
            onChange={(event) => setTag(event.target.value)}
          >
            <option value="all">All tags</option>
            {tags.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
        </label>
        <button
          className={orphansOnly ? 'asset-orphan-filter is-active' : 'asset-orphan-filter'}
          type="button"
          aria-pressed={orphansOnly}
          disabled={!usageKnown}
          onClick={() => setOrphansOnly((value) => !value)}
        >
          Orphans <b>{orphanCount}</b>
        </button>
      </div>

      {(assets.isError || usages.isError) && (
        <p className="asset-library-error">
          Could not read the asset library: {api.errorMessage(assets.error ?? usages.error)}
        </p>
      )}
      {deleteError && <p className="asset-library-error">Delete failed: {deleteError}</p>}

      <div className="asset-library-body">
        <VirtualAssetGrid
          assets={filtered}
          usageByAsset={usageByAsset}
          usageKnown={usageKnown}
          selectedId={selectedId}
          loading={assets.isPending || usages.isPending}
          onSelect={setSelectedId}
        />
        <AssetDetails
          asset={selected}
          usages={selected ? (usageByAsset.get(selected.id) ?? []) : []}
          usageKnown={usageKnown}
          nodes={nodes}
          kinds={kinds}
          readOnly={readOnly}
          onJump={onJump}
          onDelete={(asset) => {
            setDeleteError(null)
            setConfirmDelete(asset)
          }}
        />
      </div>

      {confirmDelete && (
        <ConfirmSheet
          title="Delete orphaned asset?"
          body={deleteWarning(confirmDelete)}
          confirmLabel="Delete permanently"
          danger
          busy={deleteAsset.isPending}
          onCancel={() => setConfirmDelete(null)}
          onConfirm={() => void removeConfirmed()}
        />
      )}
    </main>
  )
}

function VirtualAssetGrid({
  assets,
  usageByAsset,
  usageKnown,
  selectedId,
  loading,
  onSelect,
}: {
  assets: Asset[]
  usageByAsset: Map<string, AssetUsage[]>
  usageKnown: boolean
  selectedId: string | null
  loading: boolean
  onSelect: (id: string) => void
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ width: 900, height: 650 })
  const [scrollTop, setScrollTop] = useState(0)
  const hasAssets = assets.length > 0

  useEffect(() => {
    const element = viewport.current
    if (!element) return
    const measure = () =>
      setSize({ width: Math.max(1, element.clientWidth - 24), height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [hasAssets])

  if (assets.length === 0) {
    return (
      <div className="asset-library-empty">
        <h3>{loading ? 'Reading assets…' : 'No assets match'}</h3>
        <p>
          {loading
            ? 'The library will appear as the index answers.'
            : 'Clear or change a filter to widen the library.'}
        </p>
      </div>
    )
  }

  const columns = Math.max(1, Math.floor((size.width + GAP) / (TILE_MIN + GAP)))
  const tileWidth = (size.width - GAP * (columns - 1)) / columns
  const rows = Math.ceil(assets.length / columns)
  const startRow = Math.max(0, Math.floor(scrollTop / TILE_HEIGHT) - OVERSCAN)
  const endRow = Math.min(rows, Math.ceil((scrollTop + size.height) / TILE_HEIGHT) + OVERSCAN)
  const start = startRow * columns
  const end = Math.min(assets.length, endRow * columns)

  return (
    <div
      className="asset-grid-viewport"
      ref={viewport}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div className="asset-grid" style={{ height: rows * TILE_HEIGHT }}>
        {assets.slice(start, end).map((asset, offset) => {
          const index = start + offset
          const row = Math.floor(index / columns)
          const column = index % columns
          return (
            <AssetTile
              key={asset.id}
              asset={asset}
              usageCount={usageByAsset.get(asset.id)?.length ?? 0}
              usageKnown={usageKnown}
              selected={selectedId === asset.id}
              width={tileWidth}
              top={row * TILE_HEIGHT}
              left={column * (tileWidth + GAP)}
              onSelect={() => onSelect(asset.id)}
            />
          )
        })}
      </div>
    </div>
  )
}

function AssetTile({
  asset,
  usageCount,
  usageKnown,
  selected,
  width,
  top,
  left,
  onSelect,
}: {
  asset: Asset
  usageCount: number
  usageKnown: boolean
  selected: boolean
  width: number
  top: number
  left: number
  onSelect: () => void
}) {
  const thumb = useAssetThumb(asset.id)
  return (
    <button
      className={`asset-library-tile${selected ? ' is-selected' : ''}`}
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
      type="button"
      aria-label={`Select ${kindLabel(asset.kind).toLocaleLowerCase()} asset ${asset.id}`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="asset-library-image">
        {thumb.data ? (
          <img src={convertFileSrc(thumb.data)} alt="" />
        ) : (
          <span>{thumb.isError ? 'Preview failed' : 'Loading preview…'}</span>
        )}
        {usageKnown && usageCount === 0 && <b>Orphan</b>}
      </span>
      <span className="asset-library-tile-meta">
        <b>{kindLabel(asset.kind)}</b>
        <small>
          {asset.width}×{asset.height} · {formatBytes(asset.bytes)}
        </small>
        <code>{asset.id}</code>
      </span>
    </button>
  )
}

function AssetDetails({
  asset,
  usages,
  usageKnown,
  nodes,
  kinds,
  readOnly,
  onJump,
  onDelete,
}: {
  asset: Asset | null
  usages: AssetUsage[]
  usageKnown: boolean
  nodes: NodeSummary[]
  kinds: KindIndex
  readOnly: boolean
  onJump: (id: string) => void
  onDelete: (asset: Asset) => void
}) {
  const thumb = useAssetThumb(asset?.id ?? null)
  const linkAsset = useLinkAsset()
  const [nodeId, setNodeId] = useState(nodes[0]?.id ?? '')
  const [role, setRole] = useState<AssetRole>('full_ref')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!nodes.some((node) => node.id === nodeId)) setNodeId(nodes[0]?.id ?? '')
  }, [nodeId, nodes])
  useEffect(() => {
    setRole('full_ref')
    setError(null)
  }, [asset?.id])

  if (!asset) {
    return (
      <aside className="asset-details is-empty">
        <h3>Select an asset</h3>
        <p>Its node links, roles, cover use, and safe actions will appear here.</p>
      </aside>
    )
  }

  const assetId = asset.id
  const alreadyAttached = usages.some(
    (usage) => usage.nodeId === nodeId && usage.roles.some((item) => item.role === role),
  )
  async function attach() {
    if (!nodeId || readOnly || alreadyAttached) return
    setError(null)
    try {
      await linkAsset.mutateAsync({ nodeId, assetId, role })
    } catch (reason) {
      setError(api.errorMessage(reason))
    }
  }

  return (
    <aside className="asset-details" aria-label={`Details for asset ${asset.id}`}>
      <div className="asset-details-preview">
        {thumb.data ? (
          <img src={convertFileSrc(thumb.data)} alt="Selected asset thumbnail" />
        ) : (
          <span>{thumb.isError ? 'Preview failed' : 'Loading thumbnail…'}</span>
        )}
      </div>
      <h3>{kindLabel(asset.kind)} asset</h3>
      <dl>
        <div>
          <dt>Dimensions</dt>
          <dd>
            {asset.width}×{asset.height}
          </dd>
        </div>
        <div>
          <dt>Size</dt>
          <dd>{formatBytes(asset.bytes)}</dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd>{new Date(asset.createdAt).toLocaleString()}</dd>
        </div>
        <div>
          <dt>ID</dt>
          <dd>
            <code>{asset.id}</code>
          </dd>
        </div>
      </dl>

      <section className="asset-uses">
        <h4>
          Used by {usages.length} {usages.length === 1 ? 'node' : 'nodes'}
        </h4>
        {!usageKnown ? (
          <p className="asset-orphan-note">Usage is unavailable, so orphan actions are withheld.</p>
        ) : usages.length === 0 ? (
          <p className="asset-orphan-note">Orphan — no node links and not used as a cover.</p>
        ) : (
          usages.map((usage) => (
            <div className="asset-use" key={usage.nodeId}>
              <button type="button" onClick={() => onJump(usage.nodeId)}>
                {usage.nodeName}
              </button>
              <small>{labelFor(kinds.get(usage.nodeKind), usage.nodeKind)}</small>
              <div>
                {usage.cover && <span>Cover</span>}
                {usage.roles.map((item) => (
                  <span className={item.enabled ? '' : 'is-muted'} key={item.role}>
                    {roleLabel(item.role)} · {item.weight.toFixed(2)}
                  </span>
                ))}
              </div>
              {usage.nodeTags.length > 0 && (
                <code>{usage.nodeTags.map((tag) => `#${tag}`).join(' ')}</code>
              )}
            </div>
          ))
        )}
      </section>

      <section className="asset-attach" aria-label="Attach selected asset">
        <h4>Attach as reference</h4>
        <label>
          <span>Node</span>
          <select
            value={nodeId}
            disabled={readOnly || nodes.length === 0 || linkAsset.isPending}
            onChange={(event) => setNodeId(event.target.value)}
          >
            {nodes.map((node) => (
              <option key={node.id} value={node.id}>
                {node.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Role</span>
          <select
            value={role}
            disabled={readOnly || linkAsset.isPending}
            onChange={(event) => setRole(event.target.value as AssetRole)}
          >
            {api.ASSET_ROLES.map((value) => (
              <option key={value} value={value}>
                {roleLabel(value)}
              </option>
            ))}
          </select>
        </label>
        <button
          className="btn btn-primary"
          type="button"
          disabled={readOnly || !nodeId || alreadyAttached || linkAsset.isPending}
          onClick={() => void attach()}
        >
          {linkAsset.isPending ? 'Attaching…' : alreadyAttached ? 'Already attached' : 'Attach'}
        </button>
        {error && <p className="asset-library-error">Attach failed: {error}</p>}
      </section>

      {usageKnown && usages.length === 0 ? (
        <section className="asset-delete-offer">
          <h4>Delete orphan</h4>
          <p>Permanently removes the original and thumbnail. This cannot be undone.</p>
          <button className="btn" type="button" disabled={readOnly} onClick={() => onDelete(asset)}>
            Delete…
          </button>
        </section>
      ) : usageKnown ? (
        <p className="asset-delete-blocked">
          Detach every role and clear cover use before deletion is offered.
        </p>
      ) : null}
    </aside>
  )
}

function deleteWarning(asset: Asset): string {
  const generation =
    asset.kind === 'generated'
      ? ' Its immutable generation receipt will remain, with this output shown as missing.'
      : ''
  return `This permanently removes the original image and its thumbnail. It cannot be undone.${generation}`
}

function roleLabel(role: AssetRole): string {
  return role === 'full_ref' ? 'Full reference' : `${role.charAt(0).toUpperCase()}${role.slice(1)}`
}

function kindLabel(kind: AssetKind): string {
  return kind === 'reference' ? 'Reference' : kind === 'generated' ? 'Generated' : 'Upload'
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`
  if (bytes < 1_048_576) return `${Math.round(bytes / 1_024)} KB`
  return `${(bytes / 1_048_576).toFixed(1)} MB`
}
