import { beforeEach, describe, expect, it } from 'vitest'
import {
  arrangedBoardPoint,
  boardTileVisible,
  createBoardSpatialIndex,
  DEFAULT_BOARD_VIEWPORT,
  zoomBoardAt,
} from '../lib/board'
import { syncBoardPositions, useBoard, type BoardSyncStats } from './board'

beforeEach(() => {
  localStorage.removeItem('wobu.board-layouts.v1')
  useBoard.setState({ projects: {} })
})

describe('machine-local project board layouts', () => {
  it('keeps projects separate and persists only local geometry', () => {
    const board = useBoard.getState()
    board.syncAssets('ashfall', ['a', 'b'])
    board.setAssetPosition('ashfall', 'a', { x: 91, y: -24 })
    const positions = useBoard.getState().projects.ashfall?.positions
    board.setViewport('ashfall', { x: 12, y: 30, zoom: 1.4 })
    board.syncAssets('saltmarch', ['a'])

    expect(useBoard.getState().projects.ashfall?.positions).toBe(positions)

    expect(useBoard.getState().projects.ashfall).toMatchObject({
      viewport: { x: 12, y: 30, zoom: 1.4 },
      positions: { a: { x: 91, y: -24 }, b: arrangedBoardPoint(1) },
    })
    expect(useBoard.getState().projects.saltmarch?.positions.a).toEqual(arrangedBoardPoint(0))
    const stored = localStorage.getItem('wobu.board-layouts.v1')
    expect(stored).toContain('ashfall')

    useBoard.setState({ projects: {} })
    localStorage.setItem('wobu.board-layouts.v1', stored as string)
    useBoard.persist.rehydrate()
    expect(useBoard.getState().projects.ashfall?.positions.a).toEqual({ x: 91, y: -24 })
  })

  it('puts a new image in the first free deterministic slot after a removal', () => {
    const board = useBoard.getState()
    board.syncAssets('p', ['a', 'b', 'c'])
    board.syncAssets('p', ['a', 'c'])
    board.syncAssets('p', ['a', 'c', 'new'])

    expect(useBoard.getState().projects.p?.positions).toMatchObject({
      a: arrangedBoardPoint(0),
      new: arrangedBoardPoint(1),
      c: arrangedBoardPoint(2),
    })
  })

  it('keeps arranged slots clear of freely positioned tiles without a linear scan', () => {
    const board = useBoard.getState()
    board.syncAssets('p', ['moved'])
    board.setAssetPosition('p', 'moved', { x: 200, y: 0 })
    board.syncAssets('p', ['moved', 'new'])

    expect(useBoard.getState().projects.p?.positions.new).toEqual(arrangedBoardPoint(2))
  })

  it('lays out and interactively culls 10,000 assets with bounded operation counts', () => {
    const assetIds = Array.from({ length: 10_000 }, (_, index) => `asset-${index}`)
    const layoutStats: BoardSyncStats = { existingPoints: 0, slotsExamined: 0 }
    const positions = syncBoardPositions({}, assetIds, layoutStats)

    expect(Object.keys(positions)).toHaveLength(10_000)
    expect(layoutStats).toEqual({ existingPoints: 0, slotsExamined: 10_000 })

    const index = createBoardSpatialIndex(
      assetIds.map((assetId) => ({ assetId, point: positions[assetId]! })),
    )
    const queryStats = { buckets: 0, candidates: 0 }
    for (let frame = 0; frame < 120; frame += 1) {
      const candidatesBefore = queryStats.candidates
      const visible = index.query(
        { ...DEFAULT_BOARD_VIEWPORT, y: DEFAULT_BOARD_VIEWPORT.y - frame * 3_000 },
        { width: 800, height: 600 },
        queryStats,
      )
      expect(visible.length).toBeGreaterThan(0)
      expect(visible.length).toBeLessThan(100)
      expect(queryStats.candidates - candidatesBefore).toBeLessThan(100)
    }

    // A full scan would inspect 1.2 million records across these interactions.
    expect(queryStats.candidates).toBeLessThan(assetIds.length)
    expect(queryStats.buckets).toBeLessThan(assetIds.length)
  })
})

describe('board viewport geometry', () => {
  it('keeps the world point below the cursor fixed while zooming', () => {
    const next = zoomBoardAt(DEFAULT_BOARD_VIEWPORT, 2, { x: 300, y: 220 })
    expect((300 - next.x) / next.zoom).toBe(
      (300 - DEFAULT_BOARD_VIEWPORT.x) / DEFAULT_BOARD_VIEWPORT.zoom,
    )
    expect((220 - next.y) / next.zoom).toBe(
      (220 - DEFAULT_BOARD_VIEWPORT.y) / DEFAULT_BOARD_VIEWPORT.zoom,
    )
  })

  it('culls distant tiles while retaining an overscan band', () => {
    expect(
      boardTileVisible({ x: 100, y: 100 }, DEFAULT_BOARD_VIEWPORT, { width: 800, height: 600 }),
    ).toBe(true)
    expect(
      boardTileVisible({ x: 9000, y: 9000 }, DEFAULT_BOARD_VIEWPORT, { width: 800, height: 600 }),
    ).toBe(false)
  })
})
