import { revealItemInDir } from '@tauri-apps/plugin-opener'
import * as api from '../lib/api'
import type { MenuAnchor } from '../hooks/useContextMenu'
import { report, toast } from '../store/ui'
import { ContextMenu, MenuItem, MenuSeparator } from './ContextMenu'
import { Icon } from './Icon'

/** The file actions shared by reference and generated-image detail views. */
export function ImageContextMenu<T>({
  anchor,
  onClose,
  assetId,
  originalPath,
  label,
  deleteLabel,
  deleteDisabledReason,
  onDelete,
}: {
  anchor: MenuAnchor<T>
  onClose: () => void
  assetId: string
  originalPath: string | null
  label: string
  deleteLabel: string
  deleteDisabledReason?: string | null
  onDelete: () => void
}) {
  async function copy() {
    try {
      await api.assetCopyImage(assetId)
      toast('Image copied')
    } catch (error) {
      report(error, 'Could not copy the image')
    }
  }

  async function reveal() {
    if (!originalPath) return
    try {
      await revealItemInDir(originalPath)
    } catch (error) {
      report(error, 'Could not open the file manager')
    }
  }

  return (
    <ContextMenu
      x={anchor.x}
      y={anchor.y}
      onClose={onClose}
      restoreFocus={anchor.opener}
      label={label}
    >
      <MenuItem icon={<Icon name="copy" size="sm" />} onSelect={() => void copy()}>
        Copy image
      </MenuItem>
      <MenuItem
        icon={<Icon name="folder" size="sm" />}
        disabledReason={originalPath ? null : 'The original image is unavailable.'}
        onSelect={() => void reveal()}
      >
        View in file manager
      </MenuItem>
      <MenuSeparator />
      <MenuItem
        danger
        icon={<Icon name="trash" size="sm" />}
        disabledReason={deleteDisabledReason}
        onSelect={onDelete}
      >
        {deleteLabel}
      </MenuItem>
    </ContextMenu>
  )
}
