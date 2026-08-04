import { describe, expect, it } from 'vitest'
import { isOverflowing, placeTip, TIP_GAP } from './tooltip'

const VIEWPORT = { width: 1000, height: 800 }
const TIP = { width: 120, height: 30 }

describe('placing a tooltip', () => {
  it('sits above the control it describes, centred on it', () => {
    const at = placeTip({ left: 400, top: 400, width: 40, height: 20 }, TIP, 'top', VIEWPORT)
    expect(at.placement).toBe('top')
    expect(at.top).toBe(400 - TIP.height - TIP_GAP)
    expect(at.left).toBe(400 + 20 - 60)
  })

  it('flips to the other side rather than hanging off the window', () => {
    // A control in the title bar has nothing above it, and a tooltip clamped
    // upward would cover the thing it is naming.
    const at = placeTip({ left: 400, top: 4, width: 40, height: 20 }, TIP, 'top', VIEWPORT)
    expect(at.placement).toBe('bottom')
    expect(at.top).toBe(4 + 20 + TIP_GAP)
  })

  it('flips a side placement too, which is what the mode rail needs', () => {
    const at = placeTip({ left: 960, top: 300, width: 34, height: 34 }, TIP, 'right', VIEWPORT)
    expect(at.placement).toBe('left')
    expect(at.left).toBe(960 - TIP.width - TIP_GAP)
  })

  it('slides along the cross axis instead of flipping on it', () => {
    // Hard against the left edge: still above the control, just moved right.
    const at = placeTip({ left: 2, top: 400, width: 20, height: 20 }, TIP, 'top', VIEWPORT)
    expect(at.placement).toBe('top')
    expect(at.left).toBe(6)
  })

  it('keeps a tooltip on screen when neither side fits', () => {
    const squashed = { width: 200, height: 100 }
    const at = placeTip({ left: 10, top: 10, width: 20, height: 20 }, TIP, 'top', squashed)
    expect(at.left).toBeGreaterThanOrEqual(0)
    expect(at.top).toBeGreaterThanOrEqual(0)
    expect(at.left + TIP.width).toBeLessThanOrEqual(squashed.width + TIP.width)
  })
})

describe('deciding whether text is actually cut off', () => {
  const box = (scrollWidth: number, clientWidth: number) => ({
    scrollWidth,
    clientWidth,
    scrollHeight: 20,
    clientHeight: 20,
  })

  it('says no when the text fits', () => {
    expect(isOverflowing(box(180, 180))).toBe(false)
  })

  it('allows a pixel of rounding before calling it truncated', () => {
    // Sub-pixel layout rounds `scrollWidth` up, and a tooltip that repeats a
    // string the reader can already see in full is noise on every row.
    expect(isOverflowing(box(181, 180))).toBe(false)
    expect(isOverflowing(box(182, 180))).toBe(true)
  })

  it('notices a clipped column as well as a clipped line', () => {
    expect(
      isOverflowing({ scrollWidth: 10, clientWidth: 10, scrollHeight: 60, clientHeight: 20 }),
    ).toBe(true)
  })
})
