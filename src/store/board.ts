import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import {
  arrangedBoardPoint,
  BOARD_TILE_HEIGHT,
  BOARD_TILE_WIDTH,
  BOARD_LAYOUT_COLUMNS,
  BOARD_TILE_GAP_Y,
  clampBoardZoom,
  DEFAULT_BOARD_VIEWPORT,
  type BoardPoint,
  type BoardViewport,
} from '../lib/board'

export interface BoardProjectLayout {
  viewport: BoardViewport
  positions: Record<string, BoardPoint>
}

interface BoardState {
  projects: Record<string, BoardProjectLayout>
  syncAssets: (projectId: string, assetIds: string[]) => void
  setAssetPosition: (projectId: string, assetId: string, point: BoardPoint) => void
  setViewport: (projectId: string, viewport: BoardViewport) => void
  arrange: (projectId: string, assetIds: string[]) => void
}

export const EMPTY_BOARD_LAYOUT: BoardProjectLayout = {
  viewport: DEFAULT_BOARD_VIEWPORT,
  positions: {},
}

function finite(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function cleanPoint(value: unknown): BoardPoint | null {
  if (!value || typeof value !== 'object') return null
  const point = value as Partial<BoardPoint>
  if (
    typeof point.x !== 'number' ||
    typeof point.y !== 'number' ||
    !Number.isFinite(point.x) ||
    !Number.isFinite(point.y)
  )
    return null
  return { x: point.x, y: point.y }
}

function cleanLayout(value: unknown): BoardProjectLayout {
  const raw = value && typeof value === 'object' ? (value as Partial<BoardProjectLayout>) : {}
  const camera = raw.viewport ?? DEFAULT_BOARD_VIEWPORT
  const positions: Record<string, BoardPoint> = {}
  if (raw.positions && typeof raw.positions === 'object') {
    for (const [assetId, point] of Object.entries(raw.positions)) {
      const clean = cleanPoint(point)
      if (clean) positions[assetId] = clean
    }
  }
  return {
    viewport: {
      x: finite(camera.x, DEFAULT_BOARD_VIEWPORT.x),
      y: finite(camera.y, DEFAULT_BOARD_VIEWPORT.y),
      zoom: clampBoardZoom(finite(camera.zoom, DEFAULT_BOARD_VIEWPORT.zoom)),
    },
    positions,
  }
}

function updateProject(
  state: BoardState,
  projectId: string,
  change: (layout: BoardProjectLayout) => BoardProjectLayout,
): Pick<BoardState, 'projects'> {
  // Persisted layouts are sanitised by `merge`, and every mutation below
  // writes clean values. Preserve the positions object across viewport-only
  // changes so a camera persistence tick cannot rebuild the spatial index.
  const current = state.projects[projectId] ?? EMPTY_BOARD_LAYOUT
  return { projects: { ...state.projects, [projectId]: change(current) } }
}

export interface BoardSyncStats {
  existingPoints: number
  slotsExamined: number
}

function occupiedArrangementSlots(points: BoardPoint[]): Set<number> {
  const occupied = new Set<number>()
  const rowHeight = BOARD_TILE_HEIGHT + BOARD_TILE_GAP_Y
  for (const point of points) {
    // Arranged columns are fixed and a free-dragged point can overlap at most
    // the three rows nearest its y coordinate. Fifteen checks per point is a
    // constant, unlike comparing every new tile with every existing tile.
    const nearestRow = Math.round(point.y / rowHeight)
    for (let row = Math.max(0, nearestRow - 1); row <= nearestRow + 1; row += 1) {
      for (let column = 0; column < BOARD_LAYOUT_COLUMNS; column += 1) {
        const slot = row * BOARD_LAYOUT_COLUMNS + column
        const candidate = arrangedBoardPoint(slot)
        if (
          Math.abs(point.x - candidate.x) < BOARD_TILE_WIDTH &&
          Math.abs(point.y - candidate.y) < BOARD_TILE_HEIGHT
        ) {
          occupied.add(slot)
        }
      }
    }
  }
  return occupied
}

export function syncBoardPositions(
  existing: Record<string, BoardPoint>,
  assetIds: string[],
  stats?: BoardSyncStats,
): Record<string, BoardPoint> {
  const wanted = new Set(assetIds)
  const positions: Record<string, BoardPoint> = {}
  for (const [assetId, point] of Object.entries(existing)) {
    if (wanted.has(assetId)) positions[assetId] = point
  }

  const occupied = occupiedArrangementSlots(Object.values(positions))
  if (stats) stats.existingPoints += Object.keys(positions).length
  let slot = 0
  for (const assetId of assetIds) {
    if (positions[assetId]) continue
    while (occupied.has(slot)) {
      if (stats) stats.slotsExamined += 1
      slot += 1
    }
    if (stats) stats.slotsExamined += 1
    positions[assetId] = arrangedBoardPoint(slot)
    occupied.add(slot)
    slot += 1
  }
  return positions
}

/**
 * Machine-local, per-project mood-board layout.
 *
 * This deliberately uses the webview's local storage instead of `.wobu/`.
 * Geometry is personal UI state, not canonical world data; putting it in the
 * shared folder would create a high-churn conflict surface every time two
 * collaborators pan or arrange the same board.
 */
export const useBoard = create<BoardState>()(
  persist(
    (set) => ({
      projects: {},
      syncAssets: (projectId, assetIds) =>
        set((state) =>
          updateProject(state, projectId, (layout) => {
            return { ...layout, positions: syncBoardPositions(layout.positions, assetIds) }
          }),
        ),
      setAssetPosition: (projectId, assetId, point) =>
        set((state) =>
          updateProject(state, projectId, (layout) => ({
            ...layout,
            positions: { ...layout.positions, [assetId]: cleanPoint(point) ?? { x: 0, y: 0 } },
          })),
        ),
      setViewport: (projectId, viewport) =>
        set((state) =>
          updateProject(state, projectId, (layout) => ({
            ...layout,
            viewport: cleanLayout({ viewport, positions: {} }).viewport,
          })),
        ),
      arrange: (projectId, assetIds) =>
        set((state) =>
          updateProject(state, projectId, () => ({
            viewport: DEFAULT_BOARD_VIEWPORT,
            positions: Object.fromEntries(
              assetIds.map((assetId, index) => [assetId, arrangedBoardPoint(index)]),
            ),
          })),
        ),
    }),
    {
      name: 'wobu.board-layouts.v1',
      merge: (stored, current) => {
        const raw =
          stored && typeof stored === 'object'
            ? (stored as { projects?: Record<string, unknown> })
            : {}
        const projects: Record<string, BoardProjectLayout> = {}
        if (raw.projects && typeof raw.projects === 'object') {
          for (const [projectId, layout] of Object.entries(raw.projects)) {
            projects[projectId] = cleanLayout(layout)
          }
        }
        return { ...current, projects }
      },
    },
  ),
)
