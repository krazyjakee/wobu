interface RenderLoopOptions {
  isHidden: () => boolean
  isTurntableEnabled: () => boolean
  updateControls: () => boolean
  rotate: (delta: number) => void
  render: () => void
  getDelta: () => number
  resetClock: () => void
  stopClock: () => void
  requestFrame: (callback: FrameRequestCallback) => number
  cancelFrame: (frame: number) => void
}

export function createRenderLoop(options: RenderLoopOptions) {
  let frame: number | null = null
  let disposed = false
  let wasTurning = false

  const invalidate = () => {
    if (disposed || options.isHidden() || frame !== null) return
    frame = options.requestFrame(draw)
  }

  function draw() {
    frame = null
    if (disposed || options.isHidden()) return

    const turning = options.isTurntableEnabled()
    if (turning) {
      if (!wasTurning) options.resetClock()
      options.rotate(options.getDelta())
    } else if (wasTurning) {
      options.stopClock()
    }

    const controlsChanged = options.updateControls()
    options.render()
    wasTurning = turning
    if (turning || controlsChanged) invalidate()
  }

  return {
    invalidate,
    visibilityChanged() {
      if (options.isHidden()) {
        if (frame !== null) options.cancelFrame(frame)
        frame = null
        options.stopClock()
      } else {
        options.resetClock()
        invalidate()
      }
    },
    dispose() {
      disposed = true
      if (frame !== null) options.cancelFrame(frame)
      frame = null
      options.stopClock()
    },
  }
}
