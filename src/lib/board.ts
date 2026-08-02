export const BOARD_ASSET_MIME = 'application/x-wobu-board-asset'

export const BOARD_TILE_WIDTH = 220
export const BOARD_TILE_HEIGHT = 180
export const BOARD_TILE_GAP_X = 38
export const BOARD_TILE_GAP_Y = 54
export const BOARD_LAYOUT_COLUMNS = 5
export const BOARD_ZOOM_MIN = 0.2
export const BOARD_ZOOM_MAX = 3

export interface BoardPoint {
  x: number
  y: number
}

export interface BoardViewport extends BoardPoint {
  zoom: number
}

export interface BoardSize {
  width: number
  height: number
}

export interface BoardBounds {
  left: number
  top: number
  right: number
  bottom: number
}

export interface BoardSpatialQueryStats {
  buckets: number
  candidates: number
}

export const DEFAULT_BOARD_VIEWPORT: BoardViewport = { x: 48, y: 72, zoom: 1 }

export function arrangedBoardPoint(index: number): BoardPoint {
  const safe = Math.max(0, Math.floor(index))
  return {
    x: (safe % BOARD_LAYOUT_COLUMNS) * (BOARD_TILE_WIDTH + BOARD_TILE_GAP_X),
    y: Math.floor(safe / BOARD_LAYOUT_COLUMNS) * (BOARD_TILE_HEIGHT + BOARD_TILE_GAP_Y),
  }
}

export function clampBoardZoom(value: number): number {
  if (!Number.isFinite(value)) return 1
  return Math.min(BOARD_ZOOM_MAX, Math.max(BOARD_ZOOM_MIN, value))
}

/** Zoom around the pointer rather than making the board jump under it. */
export function zoomBoardAt(
  viewport: BoardViewport,
  nextZoom: number,
  anchor: BoardPoint,
): BoardViewport {
  const zoom = clampBoardZoom(nextZoom)
  const worldX = (anchor.x - viewport.x) / viewport.zoom
  const worldY = (anchor.y - viewport.y) / viewport.zoom
  return {
    x: anchor.x - worldX * zoom,
    y: anchor.y - worldY * zoom,
    zoom,
  }
}

export function wheelCamera(
  camera: BoardViewport,
  event: { ctrlKey: boolean; metaKey: boolean; deltaX: number; deltaY: number },
  anchor: BoardPoint,
): BoardViewport {
  if (event.ctrlKey || event.metaKey) {
    return zoomBoardAt(camera, camera.zoom * Math.exp(-event.deltaY * 0.002), anchor)
  }
  return { ...camera, x: camera.x - event.deltaX, y: camera.y - event.deltaY }
}

/**
 * The board's virtualization predicate. Coordinates are in board space; the
 * overscan is in screen pixels so it remains visually consistent at any zoom.
 */
export function boardTileVisible(
  point: BoardPoint,
  viewport: BoardViewport,
  size: BoardSize,
  overscan = 260,
): boolean {
  const { left, top, right, bottom } = boardViewportBounds(viewport, size, overscan)
  return (
    point.x + BOARD_TILE_WIDTH >= left &&
    point.x <= right &&
    point.y + BOARD_TILE_HEIGHT >= top &&
    point.y <= bottom
  )
}

export function boardViewportBounds(
  viewport: BoardViewport,
  size: BoardSize,
  overscan = 260,
): BoardBounds {
  return {
    left: (-viewport.x - overscan) / viewport.zoom,
    top: (-viewport.y - overscan) / viewport.zoom,
    right: (size.width - viewport.x + overscan) / viewport.zoom,
    bottom: (size.height - viewport.y + overscan) / viewport.zoom,
  }
}

const BOARD_SPATIAL_BUCKET = 512

/**
 * Immutable spatial index for a stable set of positioned board records.
 *
 * A tile is added to every bucket its rectangle touches, so querying only the
 * viewport's buckets cannot miss a tile that starts just outside the viewport.
 */
export function createBoardSpatialIndex<T extends { point: BoardPoint }>(items: readonly T[]) {
  const buckets = new Map<string, T[]>()
  for (const item of items) {
    const left = Math.floor(item.point.x / BOARD_SPATIAL_BUCKET)
    const right = Math.floor((item.point.x + BOARD_TILE_WIDTH) / BOARD_SPATIAL_BUCKET)
    const top = Math.floor(item.point.y / BOARD_SPATIAL_BUCKET)
    const bottom = Math.floor((item.point.y + BOARD_TILE_HEIGHT) / BOARD_SPATIAL_BUCKET)
    for (let y = top; y <= bottom; y += 1) {
      for (let x = left; x <= right; x += 1) {
        const key = `${x}:${y}`
        const bucket = buckets.get(key)
        if (bucket) bucket.push(item)
        else buckets.set(key, [item])
      }
    }
  }

  return {
    query(
      viewport: BoardViewport,
      size: BoardSize,
      stats?: BoardSpatialQueryStats,
      overscan = 260,
    ): T[] {
      const bounds = boardViewportBounds(viewport, size, overscan)
      const left = Math.floor(bounds.left / BOARD_SPATIAL_BUCKET)
      const right = Math.floor(bounds.right / BOARD_SPATIAL_BUCKET)
      const top = Math.floor(bounds.top / BOARD_SPATIAL_BUCKET)
      const bottom = Math.floor(bounds.bottom / BOARD_SPATIAL_BUCKET)
      const candidates = new Set<T>()

      for (let y = top; y <= bottom; y += 1) {
        for (let x = left; x <= right; x += 1) {
          if (stats) stats.buckets += 1
          for (const item of buckets.get(`${x}:${y}`) ?? []) candidates.add(item)
        }
      }

      if (stats) stats.candidates += candidates.size
      return [...candidates].filter((item) =>
        boardTileVisible(item.point, viewport, size, overscan),
      )
    },
  }
}
