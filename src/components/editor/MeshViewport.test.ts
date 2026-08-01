import { beforeEach, describe, expect, it, vi } from 'vitest'

import { createRenderLoop } from './meshRenderLoop'

function harness() {
  let hidden = false
  let turntable = false
  let nextFrame = 1
  const frames = new Map<number, FrameRequestCallback>()
  const updateControls = vi.fn<() => boolean>(() => false)
  const render = vi.fn()
  const rotate = vi.fn()
  const resetClock = vi.fn()
  const stopClock = vi.fn()
  const cancelFrame = vi.fn((frame: number) => frames.delete(frame))
  const loop = createRenderLoop({
    isHidden: () => hidden,
    isTurntableEnabled: () => turntable,
    updateControls,
    render,
    rotate,
    getDelta: () => 0.25,
    resetClock,
    stopClock,
    requestFrame: (callback) => {
      const frame = nextFrame++
      frames.set(frame, callback)
      return frame
    },
    cancelFrame,
  })

  return {
    loop,
    frames,
    updateControls,
    render,
    rotate,
    resetClock,
    stopClock,
    cancelFrame,
    setHidden(value: boolean) {
      hidden = value
    },
    setTurntable(value: boolean) {
      turntable = value
    },
    flushFrame() {
      const entry = frames.entries().next().value
      if (!entry) throw new Error('No animation frame is scheduled')
      const [id, callback] = entry
      frames.delete(id)
      callback(performance.now())
    },
  }
}

beforeEach(() => vi.restoreAllMocks())

describe('the mesh viewport render loop', () => {
  it('renders one invalidated frame and stops when controls are settled', () => {
    const view = harness()

    view.loop.invalidate()
    view.loop.invalidate()
    expect(view.frames.size).toBe(1)
    view.flushFrame()

    expect(view.render).toHaveBeenCalledOnce()
    expect(view.updateControls).toHaveBeenCalledOnce()
    expect(view.frames.size).toBe(0)
  })

  it('keeps repainting damping changes and stops on the first settled frame', () => {
    const view = harness()
    view.updateControls.mockReturnValueOnce(true).mockReturnValueOnce(true).mockReturnValue(false)

    view.loop.invalidate()
    view.flushFrame()
    view.flushFrame()
    view.flushFrame()

    expect(view.render).toHaveBeenCalledTimes(3)
    expect(view.frames.size).toBe(0)
  })

  it('runs continuously only for turntable animation and stops after it is disabled', () => {
    const view = harness()
    view.setTurntable(true)

    view.loop.invalidate()
    view.flushFrame()
    view.flushFrame()

    expect(view.rotate).toHaveBeenCalledTimes(2)
    expect(view.rotate).toHaveBeenLastCalledWith(0.25)
    expect(view.resetClock).toHaveBeenCalledOnce()
    expect(view.frames.size).toBe(1)

    view.setTurntable(false)
    view.flushFrame()
    expect(view.stopClock).toHaveBeenCalledOnce()
    expect(view.frames.size).toBe(0)
  })

  it('pauses while hidden and resumes turntable frames without a stale clock delta', () => {
    const view = harness()
    view.setTurntable(true)
    view.loop.invalidate()
    view.flushFrame()
    expect(view.frames.size).toBe(1)

    view.setHidden(true)
    view.loop.visibilityChanged()
    expect(view.cancelFrame).toHaveBeenCalledOnce()
    expect(view.frames.size).toBe(0)

    view.loop.invalidate()
    expect(view.frames.size).toBe(0)

    view.setHidden(false)
    view.loop.visibilityChanged()
    expect(view.resetClock).toHaveBeenCalledTimes(2)
    expect(view.frames.size).toBe(1)
    view.flushFrame()
    expect(view.rotate).toHaveBeenCalledTimes(2)
    expect(view.frames.size).toBe(1)
  })

  it('cancels a pending frame and rejects later invalidations after disposal', () => {
    const view = harness()
    view.loop.invalidate()

    view.loop.dispose()
    expect(view.cancelFrame).toHaveBeenCalledOnce()
    expect(view.stopClock).toHaveBeenCalledOnce()
    view.loop.invalidate()
    expect(view.frames.size).toBe(0)
  })
})
