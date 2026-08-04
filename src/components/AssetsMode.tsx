import { useMemo, useState } from 'react'
import * as api from '../lib/api'
import type { Asset, AssetKind, AssetRole, AssetUsage, NodeSummary } from '../lib/api'
import type { KindIndex } from '../lib/kinds'
import { labelFor } from '../lib/kinds'
import { useAssets, useAssetUsages, useDeleteAsset, useLinkAsset } from '../lib/queries'
import { useUI } from '../store/ui'
import { Combobox } from './Combobox'
import { ConfirmSheet } from './ConfirmSheet'
import { LazyAssetThumbnail } from './AssetMedia'
import { TipButton } from './Tooltip'
import { useVirtualCardWindow } from './useVirtualCardWindow'

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
  /*
   * The four filter lists.
   *
   * Kinds and roles keep their declared order — `ASSET_ROLES` is the sequence
   * the rest of the application prints them in — while nodes and tags are
   * sorted by the shared title rule, because those are names the user is
   * scanning for rather than a fixed vocabulary they already know. Each list
   * leads with its own "all" row, pinned so the way out of the filter never
   * moves.
   */
  const kindFilterOptions = useMemo(
    () => [
      { value: 'all', label: 'All kinds', pinned: true },
      ...KINDS.map((value) => ({ value, label: kindLabel(value) })),
    ],
    [],
  )
  const roleFilterOptions = useMemo(
    () => [
      { value: 'all', label: 'All roles', pinned: true },
      ...api.ASSET_ROLES.map((value) => ({ value, label: roleLabel(value) })),
    ],
    [],
  )
  const nodeFilterOptions = useMemo(
    () => [
      { value: 'all', label: 'All nodes', pinned: true },
      ...nodes.map((node) => ({ value: node.id, label: node.name, keywords: node.kind })),
    ],
    [nodes],
  )
  const tagFilterOptions = useMemo(
    () => [
      { value: 'all', label: 'All tags', pinned: true },
      ...tags.map((value) => ({ value, label: value })),
    ],
    [tags],
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
  const visibleSelectedId = filtered.some((asset) => asset.id === selectedId) ? selectedId : null
  const selected = filtered.find((asset) => asset.id === visibleSelectedId) ?? null

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
          <Combobox
            label="Filter assets by kind"
            value={kind}
            options={kindFilterOptions}
            onChange={(next) => setKind(next as AssetKind | 'all')}
          />
        </label>
        <label>
          <span>Role</span>
          <Combobox
            label="Filter assets by role"
            value={role}
            options={roleFilterOptions}
            disabled={!usageKnown}
            onChange={(next) => setRole(next as AssetRole | 'all')}
          />
        </label>
        <label>
          <span>Node</span>
          <Combobox
            label="Filter assets by node"
            value={nodeId}
            options={nodeFilterOptions}
            sort="title"
            disabled={!usageKnown}
            onChange={setNodeId}
          />
        </label>
        <label>
          <span>Linked-node tag</span>
          <Combobox
            label="Filter assets by linked node tag"
            value={tag}
            options={tagFilterOptions}
            sort="title"
            disabled={!usageKnown}
            onChange={setTag}
          />
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
        <p className="asset-library-error inline-error">
          Could not read the asset library: {api.errorMessage(assets.error ?? usages.error)}
        </p>
      )}
      {deleteError && (
        <p className="asset-library-error inline-error">Delete failed: {deleteError}</p>
      )}

      <div className="asset-library-body">
        <VirtualAssetGrid
          assets={filtered}
          usageByAsset={usageByAsset}
          usageKnown={usageKnown}
          selectedId={visibleSelectedId}
          loading={assets.isPending || usages.isPending}
          onSelect={setSelectedId}
        />
        <AssetDetails
          key={selected?.id ?? 'none'}
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
  const { viewportRef, start, end, tileWidth, totalHeight, onScroll, position } =
    useVirtualCardWindow({
      count: assets.length,
      tileMin: TILE_MIN,
      tileHeight: TILE_HEIGHT,
      gap: GAP,
      overscan: OVERSCAN,
      initialWidth: 900,
      initialHeight: 650,
    })

  if (assets.length === 0) {
    return (
      <div className="asset-library-empty empty-state">
        <h3>{loading ? 'Reading assets…' : 'No assets match'}</h3>
        <p>
          {loading
            ? 'The library will appear as the index answers.'
            : 'Clear or change a filter to widen the library.'}
        </p>
      </div>
    )
  }

  return (
    <div className="asset-grid-viewport" ref={viewportRef} onScroll={onScroll}>
      <div className="asset-grid" style={{ height: totalHeight }}>
        {assets.slice(start, end).map((asset, offset) => {
          const index = start + offset
          const cardPosition = position(index)
          return (
            <AssetTile
              key={asset.id}
              asset={asset}
              usageCount={usageByAsset.get(asset.id)?.length ?? 0}
              usageKnown={usageKnown}
              selected={selectedId === asset.id}
              width={tileWidth}
              top={cardPosition.top}
              left={cardPosition.left}
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
  return (
    <button
      className={`asset-library-tile media-card selectable-media-card${selected ? ' is-selected' : ''}`}
      style={{ width, height: TILE_HEIGHT - GAP, transform: `translate(${left}px, ${top}px)` }}
      type="button"
      aria-label={`Select ${kindLabel(asset.kind).toLocaleLowerCase()} asset ${asset.id}`}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span className="asset-library-image asset-media-frame">
        <LazyAssetThumbnail
          assetId={asset.id}
          alt=""
          loadingLabel="Loading preview…"
          missingLabel="Preview failed"
          errorLabel="Preview failed"
        />
        {usageKnown && usageCount === 0 && <b>Orphan</b>}
      </span>
      <span className="asset-library-tile-meta media-card-copy">
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
  const linkAsset = useLinkAsset()
  const [selectedNodeId, setNodeId] = useState(nodes[0]?.id ?? '')
  const [role, setRole] = useState<AssetRole>('full_ref')
  const [error, setError] = useState<string | null>(null)

  const nodeId = nodes.some((node) => node.id === selectedNodeId)
    ? selectedNodeId
    : (nodes[0]?.id ?? '')
  // Declared above the early return: this panel renders an empty state when
  // nothing is selected, and hooks cannot sit behind that branch.
  const attachNodeOptions = useMemo(
    () => nodes.map((node) => ({ value: node.id, label: node.name, keywords: node.kind })),
    [nodes],
  )
  const attachRoleOptions = useMemo(
    () => api.ASSET_ROLES.map((value) => ({ value, label: roleLabel(value) })),
    [],
  )

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
        <LazyAssetThumbnail
          assetId={asset.id}
          alt="Selected asset thumbnail"
          loadingLabel="Loading thumbnail…"
          missingLabel="Preview failed"
          errorLabel="Preview failed"
        />
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
          <Combobox
            label="Node"
            value={nodeId}
            options={attachNodeOptions}
            sort="title"
            placeholder="No nodes"
            disabled={readOnly || nodes.length === 0 || linkAsset.isPending}
            onChange={setNodeId}
          />
        </label>
        <label>
          <span>Role</span>
          <Combobox
            label="Role"
            value={role}
            options={attachRoleOptions}
            disabled={readOnly || linkAsset.isPending}
            onChange={(next) => setRole(next as AssetRole)}
          />
        </label>
        <TipButton
          className="btn btn-primary"
          disabledReason={
            readOnly
              ? 'This project is open read-only, so nothing can be attached to it.'
              : !nodeId
                ? 'Choose the node this picture is a reference for.'
                : alreadyAttached
                  ? 'This asset is already attached to that node in that role.'
                  : linkAsset.isPending
                    ? 'The last attach is still being written.'
                    : null
          }
          tip="Link this asset to the node as a generation reference"
          onClick={() => void attach()}
        >
          {linkAsset.isPending ? 'Attaching…' : alreadyAttached ? 'Already attached' : 'Attach'}
        </TipButton>
        {error && <p className="asset-library-error inline-error">Attach failed: {error}</p>}
      </section>

      {usageKnown && usages.length === 0 ? (
        <section className="asset-delete-offer">
          <h4>Delete orphan</h4>
          <p>Permanently removes the original and thumbnail. This cannot be undone.</p>
          <TipButton
            className="btn"
            disabledReason={
              readOnly ? 'This project is open read-only, so nothing in it can be deleted.' : null
            }
            tip="Delete the original and its thumbnail from the project folder"
            onClick={() => onDelete(asset)}
          >
            Delete…
          </TipButton>
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
