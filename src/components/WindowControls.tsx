import { useEffect, useState } from 'react'
import { Icon } from './Icon'
import { IconButton } from './Tooltip'
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

  // `placement="bottom"`: these sit in the title bar, and a tooltip above them
  // would be off the top of the window rather than under the pointer.
  return (
    <div className="wctl">
      <IconButton label="Minimise" placement="bottom" onClick={() => void minimizeWindow()}>
        <Icon name="win-min" />
      </IconButton>
      <IconButton
        label={max ? 'Restore' : 'Maximise'}
        placement="bottom"
        onClick={() => void toggleMaximizeWindow()}
      >
        <Icon name={max ? 'win-restore' : 'win-max'} />
      </IconButton>
      <IconButton
        className="close"
        label="Close"
        placement="bottom"
        onClick={() => void closeWindow()}
      >
        <Icon name="x" />
      </IconButton>
    </div>
  )
}
