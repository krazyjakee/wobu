import { useEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties, DragEvent, WheelEvent } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '../lib/api'
import type { Asset, AssetRole, NodeSummary } from '../lib/api'
import {
  arrangedBoardPoint,
  BOARD_ASSET_MIME,
  BOARD_TILE_HEIGHT,
  BOARD_TILE_WIDTH,
  boardTileVisible,
  DEFAULT_BOARD_VIEWPORT,
  zoomBoardAt,
  type BoardPoint,
  type BoardSize,
  type BoardViewport,
} from '../lib/board'
import { colorFor, labelFor, spriteFor, type KindIndex } from '../lib/kinds'
import { useAssets, useAssetThumb, useLinkAsset } from '../lib/queries'
import { EMPTY_BOARD_LAYOUT, useBoard } from '../store/board'
import { toast, useUI } from '../store/ui'
import { Icon } from './Icon'

export interface BoardAttachRequest {
  assetId: string
  nodeId: string
}

export function BoardMode({
  projectId,
  nodes,
  kinds,
  readOnly,
  navigatorVisible,
  pendingAttach,
  onPendingAttach,
}: {
  projectId: string
  nodes: NodeSummary[]
  kinds: KindIndex
  readOnly: boolean
  navigatorVisible: boolean
  pendingAttach: BoardAttachRequest | null
  onPendingAttach: (request: BoardAttachRequest | null) => void
}) {
  const setMode = useUI((state) => state.setMode)
  const selectedId = useUI((state) => state.selectedId)
  const assets = useAssets(true)
  const layout = useBoard((state) => state.projects[projectId] ?? EMPTY_BOARD_LAYOUT)
  const syncAssets = useBoard((state) => state.syncAssets)
  const setAssetPosition = useBoard((state) => state.setAssetPosition)
  const persistViewport = useBoard((state) => state.setViewport)
  const arrange = useBoard((state) => state.arrange)
  const viewportRef = useRef<HTMLDivElement>(null)
  const pan = useRef<{
    pointerId: number
    start: BoardPoint
    origin: BoardViewport
  } | null>(null)
  const [size, setSize] = useState<BoardSize>({ width: 1200, height: 700 })
  const [camera, setCamera] = useState(layout.viewport)
  const [isPanning, setIsPanning] = useState(false)
  const assetList = useMemo(() => assets.data ?? [], [assets.data])
  const selectedNode = nodes.find((node) => node.id === selectedId) ?? null

  useEffect(() => {
    if (assets.isSuccess)
      syncAssets(
        projectId,
        assetList.map((asset) => asset.id),
      )
  }, [assetList, assets.isSuccess, projectId, syncAssets])

  useEffect(() => {
    setCamera(layout.viewport)
  }, [projectId, layout.viewport.x, layout.viewport.y, layout.viewport.zoom])

  // Camera changes are high-frequency while panning. Persist them after the
  // gesture pauses so localStorage is not rewritten for every pointer event.
  useEffect(() => {
    const timer = window.setTimeout(() => persistViewport(projectId, camera), 120)
    return () => window.clearTimeout(timer)
  }, [camera, persistViewport, projectId])

  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const positioned = useMemo(
    () =>
      assetList.map((asset, index) => ({
        asset,
        point: layout.positions[asset.id] ?? arrangedBoardPoint(index),
      })),
    [assetList, layout.positions],
  )
  const visible = useMemo(
    () => positioned.filter((item) => boardTileVisible(item.point, camera, size)),
    [camera, positioned, size],
  )

  function localPoint(clientX: number, clientY: number): BoardPoint {
    const rect = viewportRef.current?.getBoundingClientRect()
    return { x: clientX - (rect?.left ?? 0), y: clientY - (rect?.top ?? 0) }
  }

  function moveAsset(assetId: string, clientX: number, clientY: number) {
    const local = localPoint(clientX, clientY)
    setAssetPosition(projectId, assetId, {
      x: (local.x - camera.x) / camera.zoom - BOARD_TILE_WIDTH / 2,
      y: (local.y - camera.y) / camera.zoom - BOARD_TILE_HEIGHT / 2,
    })
  }

  function acceptCanvasDrop(event: DragEvent<HTMLDivElement>) {
    const assetId = boardAssetFrom(event)
    if (!assetId) return
    event.preventDefault()
    moveAsset(assetId, event.clientX, event.clientY)
  }

  function zoomBy(factor: number) {
    setCamera((current) =>
      zoomBoardAt(current, current.zoom * factor, { x: size.width / 2, y: size.height / 2 }),
    )
  }

  return (
    <main
      className="board-mode"
      style={{ gridColumn: navigatorVisible ? '3 / -1' : '2 / -1' }}
      aria-label="Mood board"
    >
      <header className="board-head">
        <div>
          <h2>Board</h2>
          <p>
            {assetList.length} images · layout stays on this machine · drag a picture onto a node
          </p>
        </div>
        <div className="board-actions">
          <button
            className="btn"
            type="button"
            onClick={() =>
              arrange(
                projectId,
                assetList.map((asset) => asset.id),
              )
            }
          >
            Arrange
          </button>
          <button className="btn" type="button" aria-label="Zoom out" onClick={() => zoomBy(0.8)}>
            −
          </button>
          <output aria-label="Board zoom">{Math.round(camera.zoom * 100)}%</output>
          <button className="btn" type="button" aria-label="Zoom in" onClick={() => zoomBy(1.25)}>
            +
          </button>
          <button className="btn" type="button" onClick={() => setCamera(DEFAULT_BOARD_VIEWPORT)}>
            Reset view
          </button>
          <button className="btn" type="button" onClick={() => setMode('library')}>
            Back to Library
          </button>
        </div>
      </header>

      <div
        className={`board-viewport${isPanning ? ' is-panning' : ''}`}
        ref={viewportRef}
        onDragOver={(event) => {
          if (!hasBoardAsset(event)) return
          event.preventDefault()
          event.dataTransfer.dropEffect = 'move'
        }}
        onDrop={acceptCanvasDrop}
        onWheel={(event) => {
          event.preventDefault()
          const anchor = localPoint(event.clientX, event.clientY)
          setCamera((current) => wheelCamera(current, event, anchor))
        }}
        onPointerDown={(event) => {
          if (event.button !== 0 || interactiveTarget(event.target)) return
          pan.current = {
            pointerId: event.pointerId,
            start: { x: event.clientX, y: event.clientY },
            origin: camera,
          }
          setIsPanning(true)
          event.currentTarget.setPointerCapture?.(event.pointerId)
        }}
        onPointerMove={(event) => {
          const gesture = pan.current
          if (!gesture || gesture.pointerId !== event.pointerId) return
          setCamera({
            ...gesture.origin,
            x: gesture.origin.x + event.clientX - gesture.start.x,
            y: gesture.origin.y + event.clientY - gesture.start.y,
          })
        }}
        onPointerUp={(event) => {
          if (pan.current?.pointerId !== event.pointerId) return
          pan.current = null
          setIsPanning(false)
          event.currentTarget.releasePointerCapture?.(event.pointerId)
        }}
        onPointerCancel={() => {
          pan.current = null
          setIsPanning(false)
        }}
      >
        {assets.isError && (
          <p className="board-error">
            Could not read the asset library: {api.errorMessage(assets.error)}
          </p>
        )}
        {selectedNode && (
          <NodeDropChip
            node={selectedNode}
            kinds={kinds}
            readOnly={readOnly}
            onAsset={(assetId) => onPendingAttach({ assetId, nodeId: selectedNode.id })}
          />
        )}
        {!selectedNode && nodes.length > 0 && (
          <p className="board-node-hint" data-board-interactive>
            Select a node in the navigator to add its drop target to the canvas.
          </p>
        )}

        {assetList.length === 0 && !assets.isPending && !assets.isError && (
          <div className="board-empty" data-board-interactive>
            <h3>No images yet</h3>
            <p>Import references or generate concepts, then they will appear here automatically.</p>
          </div>
        )}
        {assets.isPending && <p className="board-loading">Reading the board…</p>}

        <div
          className="board-world"
          style={{ transform: `translate(${camera.x}px, ${camera.y}px) scale(${camera.zoom})` }}
        >
          {visible.map(({ asset, point }) => (
            <BoardAssetTile key={asset.id} asset={asset} point={point} />
          ))}
        </div>
        <div className="board-cull-count" aria-live="polite">
          {visible.length} of {assetList.length} images in view
        </div>
      </div>

      {pendingAttach && (
        <AttachReferenceSheet
          request={pendingAttach}
          asset={assetList.find((asset) => asset.id === pendingAttach.assetId) ?? null}
          node={nodes.find((node) => node.id === pendingAttach.nodeId) ?? null}
          readOnly={readOnly}
          onClose={() => onPendingAttach(null)}
        />
      )}
    </main>
  )
}

function BoardAssetTile({ asset, point }: { asset: Asset; point: BoardPoint }) {
  const thumb = useAssetThumb(asset.id)
  return (
    <article
      className="board-asset"
      style={{ left: point.x, top: point.y }}
      draggable
      data-board-interactive
      aria-label={`${kindLabel(asset.kind)} asset ${asset.id}`}
      onDragStart={(event) => {
        event.dataTransfer.setData(BOARD_ASSET_MIME, asset.id)
        event.dataTransfer.effectAllowed = 'move'
      }}
    >
      <div className="board-asset-image">
        {thumb.data ? (
          <img src={convertFileSrc(thumb.data)} alt="" draggable={false} />
        ) : (
          <span>{thumb.isError ? 'Preview failed' : 'Loading preview…'}</span>
        )}
      </div>
      <footer>
        <b>{kindLabel(asset.kind)}</b>
        <code>{asset.id}</code>
      </footer>
    </article>
  )
}

function NodeDropChip({
  node,
  kinds,
  readOnly,
  onAsset,
}: {
  node: NodeSummary
  kinds: KindIndex
  readOnly: boolean
  onAsset: (assetId: string) => void
}) {
  const def = kinds.get(node.kind)
  return (
    <div
      className="board-node-chip"
      data-board-interactive
      style={{ '--board-node-color': colorFor(def, node.kind) } as CSSProperties}
      onDragOver={(event) => {
        if (readOnly || !hasBoardAsset(event)) return
        event.preventDefault()
        event.stopPropagation()
        event.dataTransfer.dropEffect = 'link'
      }}
      onDrop={(event) => {
        if (readOnly) return
        const assetId = boardAssetFrom(event)
        if (!assetId) return
        event.preventDefault()
        event.stopPropagation()
        onAsset(assetId)
      }}
    >
      <Icon name={spriteFor(def, node.kind)} size="sm" />
      <span>
        <small>Drop on selected node</small>
        <b>{node.name}</b>
      </span>
      <em>{readOnly ? 'Read only' : labelFor(def, node.kind)}</em>
    </div>
  )
}

function AttachReferenceSheet({
  request,
  asset,
  node,
  readOnly,
  onClose,
}: {
  request: BoardAttachRequest
  asset: Asset | null
  node: NodeSummary | null
  readOnly: boolean
  onClose: () => void
}) {
  const link = useLinkAsset()
  const [role, setRole] = useState<AssetRole>('mood')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setRole('mood')
    setError(null)
  }, [request.assetId, request.nodeId])

  async function attach() {
    if (!asset || !node || readOnly) return
    setError(null)
    try {
      await link.mutateAsync({ nodeId: node.id, assetId: asset.id, role })
      toast(`${assetLabel(asset)} attached to ${node.name} as ${roleLabel(role)}.`)
      onClose()
    } catch (reason) {
      setError(api.errorMessage(reason))
    }
  }

  return (
    <div
      className="scrim"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        className="sheet board-attach-sheet"
        role="dialog"
        aria-modal="true"
        aria-label="Attach board image"
      >
        <h2>Attach board image</h2>
        {asset && node ? (
          <>
            <p>
              Choose how <b>{assetLabel(asset)}</b> should influence <b>{node.name}</b>.
            </p>
            <label>
              <span>Reference role</span>
              <select
                aria-label="Reference role"
                value={role}
                disabled={readOnly || link.isPending}
                onChange={(event) => setRole(event.target.value as AssetRole)}
                autoFocus
              >
                {api.ASSET_ROLES.map((value) => (
                  <option key={value} value={value}>
                    {roleLabel(value)}
                  </option>
                ))}
              </select>
            </label>
            <p className="board-role-note">
              Mood references are never sent to providers; other roles may be routed to compatible
              image backends.
            </p>
          </>
        ) : (
          <p>The image or node no longer exists. Close this sheet and choose another target.</p>
        )}
        {error && <p className="board-error">Attach failed: {error}</p>}
        <div className="sheet-actions">
          <button className="btn btn-ghost" type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            type="button"
            disabled={!asset || !node || readOnly || link.isPending}
            onClick={() => void attach()}
          >
            {link.isPending ? 'Attaching…' : 'Attach reference'}
          </button>
        </div>
      </div>
    </div>
  )
}

function hasBoardAsset(event: DragEvent<HTMLElement>): boolean {
  return Array.from(event.dataTransfer.types).includes(BOARD_ASSET_MIME)
}

function boardAssetFrom(event: DragEvent<HTMLElement>): string | null {
  if (!hasBoardAsset(event)) return null
  return event.dataTransfer.getData(BOARD_ASSET_MIME) || null
}

function interactiveTarget(target: EventTarget): boolean {
  return target instanceof Element && !!target.closest('[data-board-interactive]')
}

function roleLabel(role: AssetRole): string {
  return role === 'full_ref' ? 'Full reference' : `${role.charAt(0).toUpperCase()}${role.slice(1)}`
}

function kindLabel(kind: Asset['kind']): string {
  return kind === 'reference' ? 'Reference' : kind === 'generated' ? 'Generated' : 'Upload'
}

function assetLabel(asset: Asset): string {
  return `${kindLabel(asset.kind).toLocaleLowerCase()} image ${asset.id}`
}

export function wheelCamera(
  camera: BoardViewport,
  event: Pick<WheelEvent, 'ctrlKey' | 'metaKey' | 'deltaX' | 'deltaY'>,
  anchor: BoardPoint,
): BoardViewport {
  if (event.ctrlKey || event.metaKey) {
    return zoomBoardAt(camera, camera.zoom * Math.exp(-event.deltaY * 0.002), anchor)
  }
  return { ...camera, x: camera.x - event.deltaX, y: camera.y - event.deltaY }
}
