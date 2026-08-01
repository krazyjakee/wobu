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
  const left = (-viewport.x - overscan) / viewport.zoom
  const top = (-viewport.y - overscan) / viewport.zoom
  const right = (size.width - viewport.x + overscan) / viewport.zoom
  const bottom = (size.height - viewport.y + overscan) / viewport.zoom
  return (
    point.x + BOARD_TILE_WIDTH >= left &&
    point.x <= right &&
    point.y + BOARD_TILE_HEIGHT >= top &&
    point.y <= bottom
  )
}
