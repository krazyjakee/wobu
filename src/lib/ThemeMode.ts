import { getCurrentWindow } from '@tauri-apps/api/window'

/**
 * Which palette the window is wearing (issue #134).
 *
 * The whole mechanism is one attribute: `<html data-theme="light|dark">`, which
 * `styles/tokens.css` answers with a different set of custom properties. Nothing
 * else in the app knows a theme exists — a component that wants a colour asks
 * for a role, and the role has already resolved.
 *
 * "System" is stored as `system` rather than resolved once and written down. A
 * preference that recorded `dark` because that is what the desktop was on the
 * evening it was chosen would stop following the desktop, which is the entire
 * behaviour the option promises.
 */

export type ThemeMode = 'system' | 'light' | 'dark'
/** What `system` becomes once the OS has been asked. */
export type ResolvedTheme = 'light' | 'dark'

export const THEME_MODES: readonly ThemeMode[] = ['system', 'light', 'dark']
export const DEFAULT_THEME: ThemeMode = 'system'

/** The label the settings pane prints, and the one the tests assert on. */
export const THEME_LABELS: Record<ThemeMode, string> = {
  system: 'Match system',
  light: 'Light',
  dark: 'Dark',
}

const LIGHT_QUERY = '(prefers-color-scheme: light)'

export function isThemeMode(value: unknown): value is ThemeMode {
  return typeof value === 'string' && (THEME_MODES as readonly string[]).includes(value)
}

/**
 * Whether the desktop is asking for light.
 *
 * Queried for *light* rather than dark so that an environment with no colour
 * preference at all — an old WebView, a headless jsdom run — comes back false
 * and lands on dark, which is the theme Wobu has always shipped and the one the
 * boot screen paints when it cannot know better.
 */
export function prefersLight(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(LIGHT_QUERY).matches
    : false
}

export function resolveTheme(mode: ThemeMode, systemPrefersLight = prefersLight()): ResolvedTheme {
  if (mode === 'light' || mode === 'dark') return mode
  return systemPrefersLight ? 'light' : 'dark'
}

/**
 * Put the resolved theme on `<html>` and tell the OS window about it.
 *
 * The attribute is written even when it has not changed: this is also what runs
 * on first evaluation of the bundle, and at that point the document is carrying
 * whatever `index.html` decided from `prefers-color-scheme` alone.
 */
export function applyTheme(mode: ThemeMode): ResolvedTheme {
  const resolved = resolveTheme(mode)
  if (typeof document !== 'undefined') document.documentElement.dataset.theme = resolved
  syncWindowTheme(resolved)
  return resolved
}

/**
 * Match the native window chrome to the webview.
 *
 * Decorations are off, so this is not about the title bar: it is the colour the
 * compositor paints while the window resizes past what the webview has drawn,
 * and the theme native menus and scrollbars inherit.
 *
 * Best-effort on purpose. `capabilities/default.json` does not grant
 * `core:window:allow-set-theme`, so today this rejects and the CSS is what the
 * user sees; adding the permission is a one-line change in a part of the tree
 * this issue does not own, and when it lands this starts working with no change
 * here. It is also a no-op in a browser, which is how the frontend is served for
 * screenshots.
 */
function syncWindowTheme(resolved: ResolvedTheme): void {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return
  try {
    void getCurrentWindow()
      .setTheme(resolved)
      .catch(() => {})
  } catch {
    /* No window to talk to. The CSS has already done the visible work. */
  }
}

/**
 * Call `onChange` whenever the desktop switches between light and dark.
 *
 * Returns an unsubscribe, and is a no-op where `matchMedia` is missing. The
 * listener stays attached for every mode rather than only `system`, because the
 * cost is one event and the alternative is a subscription that has to be torn
 * down and rebuilt every time the preference changes.
 */
export function watchSystemTheme(onChange: (systemPrefersLight: boolean) => void): () => void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return () => {}
  const query = window.matchMedia(LIGHT_QUERY)
  const listener = (event: MediaQueryListEvent) => onChange(event.matches)
  // `addListener` is the deprecated form Safari carried for years; some WebView
  // versions Tauri sits on are old enough to have only that one.
  if (typeof query.addEventListener === 'function') {
    query.addEventListener('change', listener)
    return () => query.removeEventListener('change', listener)
  }
  query.addListener(listener)
  return () => query.removeListener(listener)
}
