import { useEffect, useState } from 'react'
import { Icon } from './Icon'
import {
  closeWindow,
  isMaximized,
  minimizeWindow,
  onResized,
  toggleMaximizeWindow,
} from '../lib/window'

export function WindowControls() {
  const [max, setMax] = useState(false)

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    const sync = () => {
      void isMaximized().then((v) => {
        if (!disposed) setMax(v)
      })
    }
    sync()
    void onResized(sync).then((fn) => {
      if (disposed) fn()
      else unlisten = fn
    })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  return (
    <div className="wctl">
      <button onClick={() => void minimizeWindow()} title="Minimise" aria-label="Minimise">
        <Icon name="win-min" />
      </button>
      <button
        onClick={() => void toggleMaximizeWindow()}
        title={max ? 'Restore' : 'Maximise'}
        aria-label={max ? 'Restore' : 'Maximise'}
      >
        <Icon name={max ? 'win-restore' : 'win-max'} />
      </button>
      <button className="close" onClick={() => void closeWindow()} title="Close" aria-label="Close">
        <Icon name="x" />
      </button>
    </div>
  )
}
