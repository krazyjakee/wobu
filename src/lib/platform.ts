export function modKey(): string {
  const mac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
  return mac ? '⌘' : 'Ctrl+'
}
