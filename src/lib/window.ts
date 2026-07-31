import { getCurrentWindow } from '@tauri-apps/api/window'
import { isTauri } from './api'

/**
 * Tauri decorations are off, so the title bar draws its own controls.
 * Outside the webview these are no-ops rather than crashes.
 */

export async function minimizeWindow() {
  if (!isTauri()) return
  await getCurrentWindow().minimize()
}

export async function toggleMaximizeWindow() {
  if (!isTauri()) return
  await getCurrentWindow().toggleMaximize()
}

export async function closeWindow() {
  if (!isTauri()) return
  await getCurrentWindow().close()
}

export async function isMaximized(): Promise<boolean> {
  if (!isTauri()) return false
  return getCurrentWindow().isMaximized()
}

/** Fires whenever the window is resized, so the max/restore glyph stays honest. */
export async function onResized(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {}
  return getCurrentWindow().onResized(() => cb())
}
