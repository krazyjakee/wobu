export function isMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
}

export function modKey(): string {
  return isMac() ? '⌘' : 'Ctrl+'
}
