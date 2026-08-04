import { useMemo, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Asset, AssetKind, AssetRole, AssetUsage, NodeSummary } from '../lib/api'
import type { KindIndex } from '../lib/kinds'
import { labelFor } from '../lib/kinds'
import { useAssets, useAssetUsages, useDeleteAsset, useLinkAsset } from '../lib/queries'
import { useUI } from '../store/ui'
import { Combobox } from './Combobox'
import { ConfirmSheet } from './ConfirmSheet'
import { LazyAssetThumbnail } from './AssetMedia'
import { Modal } from './Modal'
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
      { value: 'all', label: 'All entities', pinned: true },
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
            {assets.data?.length ?? 0} images · {orphanCount} unused · previews load as you scroll
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
          <span>Entity</span>
          <Combobox
            label="Filter assets by entity"
            value={nodeId}
            options={nodeFilterOptions}
            sort="title"
            disabled={!usageKnown}
            onChange={setNodeId}
          />
        </label>
        <label>
          <span>Tag on a linked entity</span>
          <Combobox
            label="Filter assets by a tag on a linked entity"
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
          Unused <b>{orphanCount}</b>
        </button>
      </div>

      {(assets.isError || usages.isError) && (
        <p className="asset-library-error inline-error">
          Could not read the assets: {api.errorMessage(assets.error ?? usages.error)}
        </p>
      )}
      {deleteError && (
        <p className="asset-library-error inline-error">Could not delete it: {deleteError}</p>
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
          title="Delete this unused image?"
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
            ? 'The assets will appear as Wobu finishes reading the project.'
            : 'Clear or change a filter to see more.'}
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
        {usageKnown && usageCount === 0 && <b>Unused</b>}
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
  const [fullSize, setFullSize] = useState<string | null>(null)
  const [opening, setOpening] = useState(false)
  const [openError, setOpenError] = useState<string | null>(null)

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
        <p>
          Which entities use it, in what role, whether it is a cover, and what you can safely do
          with it all appear here.
        </p>
      </aside>
    )
  }

  const assetId = asset.id
  const alreadyAttached = usages.some(
    (usage) => usage.nodeId === nodeId && usage.roles.some((item) => item.role === role),
  )
  /*
   * The original, read on the click that asks for it.
   *
   * Everything above this panel is drawn from thumbnails the grid already has,
   * which is what keeps a library of thousands cheap to scroll. The original is
   * the megabytes: it is fetched here, for one asset, and never as part of
   * selecting a tile. `asset_original` answers `null` when the blob has gone
   * missing from the folder — a thing to say under the preview, not an error.
   */
  async function openFullSize() {
    setOpening(true)
    setOpenError(null)
    try {
      const path = await api.assetOriginal(assetId)
      if (path) setFullSize(convertFileSrc(path))
      else setOpenError('The original is no longer in the project folder.')
    } catch (reason) {
      setOpenError(api.errorMessage(reason))
    } finally {
      setOpening(false)
    }
  }

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
      <button
        className="asset-details-preview"
        type="button"
        disabled={opening}
        aria-label={`View asset ${asset.id} full size`}
        onClick={() => void openFullSize()}
      >
        <LazyAssetThumbnail
          assetId={asset.id}
          alt=""
          loadingLabel="Loading thumbnail…"
          missingLabel="Preview failed"
          errorLabel="Preview failed"
        />
        <span className="asset-details-zoom">{opening ? 'Opening…' : 'View full size'}</span>
      </button>
      {openError && (
        <p className="asset-library-error inline-error">Could not open it: {openError}</p>
      )}
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
          Used by {usages.length} {usages.length === 1 ? 'entity' : 'entities'}
        </h4>
        {!usageKnown ? (
          <p className="asset-orphan-note">
            Wobu could not work out where this is used, so it will not offer to delete it.
          </p>
        ) : usages.length === 0 ? (
          <p className="asset-orphan-note">
            Not used — no entity links to this image, and nothing uses it as a cover.
          </p>
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
          <span>Entity</span>
          <Combobox
            label="Entity"
            value={nodeId}
            options={attachNodeOptions}
            sort="title"
            placeholder="No entities"
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
              ? 'This project folder is read-only, so nothing can be attached to it.'
              : !nodeId
                ? 'Choose the entity this picture is a reference for.'
                : alreadyAttached
                  ? 'This image is already attached to that entity in that role.'
                  : linkAsset.isPending
                    ? 'The last change is still being saved.'
                    : null
          }
          tip="Attach this image to that entity as a reference for its generations"
          onClick={() => void attach()}
        >
          {linkAsset.isPending ? 'Attaching…' : alreadyAttached ? 'Already attached' : 'Attach'}
        </TipButton>
        {error && <p className="asset-library-error inline-error">Could not attach it: {error}</p>}
      </section>

      {usageKnown && usages.length === 0 ? (
        <section className="asset-delete-offer">
          <h4>Delete unused image</h4>
          <p>Permanently removes the original and thumbnail. This cannot be undone.</p>
          <TipButton
            className="btn"
            disabledReason={
              readOnly ? 'This project folder is read-only, so nothing in it can be deleted.' : null
            }
            tip="Delete the original and its thumbnail from the project folder"
            onClick={() => onDelete(asset)}
          >
            Delete…
          </TipButton>
        </section>
      ) : usageKnown ? (
        <p className="asset-delete-blocked">
          Detach it from every entity, and clear it from any covers, before Wobu will offer to
          delete it.
        </p>
      ) : null}

      {fullSize && (
        <Modal
          className="asset-viewer"
          scrimClassName="asset-viewer-scrim"
          titleId="asset-viewer-title"
          descriptionId="asset-viewer-description"
          onClose={() => setFullSize(null)}
        >
          <h2 id="asset-viewer-title" className="modal-sr-only">
            Full-size asset
          </h2>
          <p id="asset-viewer-description" className="modal-sr-only">
            The original image for asset {asset.id}. Press Escape, or use Close, to go back to the
            library.
          </p>
          <img src={fullSize} alt={`Asset ${asset.id} at full size`} />
          <button
            className="ibtn asset-viewer-close"
            type="button"
            onClick={() => setFullSize(null)}
            aria-label="Close full-size image"
            data-modal-initial-focus
          >
            ×
          </button>
        </Modal>
      )}
    </aside>
  )
}

function deleteWarning(asset: Asset): string {
  const generation =
    asset.kind === 'generated'
      ? ' The receipt for the generation that made it is kept, and will show this image as missing.'
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
