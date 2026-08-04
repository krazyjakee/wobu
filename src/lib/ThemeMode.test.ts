import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  DEFAULT_THEME,
  THEME_MODES,
  applyTheme,
  isThemeMode,
  resolveTheme,
  watchSystemTheme,
} from './ThemeMode'
import { useSettings } from '../store/settings'

/** A `matchMedia` whose answer we control, and whose `change` we can fire. */
function stubMatchMedia(prefersLight: boolean) {
  const listeners = new Set<(event: MediaQueryListEvent) => void>()
  const query = {
    matches: prefersLight,
    media: '(prefers-color-scheme: light)',
    addEventListener: (_: string, fn: (event: MediaQueryListEvent) => void) => {
      listeners.add(fn)
    },
    removeEventListener: (_: string, fn: (event: MediaQueryListEvent) => void) => {
      listeners.delete(fn)
    },
  }
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => query),
  )
  return {
    emit(matches: boolean) {
      query.matches = matches
      for (const fn of listeners) fn({ matches } as MediaQueryListEvent)
    },
    get listenerCount() {
      return listeners.size
    },
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
  delete document.documentElement.dataset.theme
})

describe('resolveTheme', () => {
  it('takes a fixed choice literally, whatever the desktop says', () => {
    expect(resolveTheme('light', false)).toBe('light')
    expect(resolveTheme('dark', true)).toBe('dark')
  })

  it('asks the desktop only for system', () => {
    expect(resolveTheme('system', true)).toBe('light')
    expect(resolveTheme('system', false)).toBe('dark')
  })

  it('falls back to dark where there is no colour preference to read', () => {
    // An old WebView, or a test runner: `(prefers-color-scheme: light)` comes
    // back false and we land on the theme Wobu has always shipped, which is
    // also the one the boot screen in index.html paints.
    vi.stubGlobal('matchMedia', undefined)
    expect(resolveTheme('system')).toBe('dark')
  })
})

describe('applyTheme', () => {
  it('writes the resolved theme onto the document element', () => {
    stubMatchMedia(true)
    expect(applyTheme('system')).toBe('light')
    expect(document.documentElement.dataset.theme).toBe('light')

    expect(applyTheme('dark')).toBe('dark')
    expect(document.documentElement.dataset.theme).toBe('dark')
  })
})

describe('the mode list', () => {
  it('accepts exactly the three modes, and nothing a stale store might hold', () => {
    for (const mode of THEME_MODES) expect(isThemeMode(mode)).toBe(true)
    expect(isThemeMode('Dark')).toBe(false)
    expect(isThemeMode(null)).toBe(false)
    expect(isThemeMode(1)).toBe(false)
    expect(THEME_MODES).toContain(DEFAULT_THEME)
  })
})

describe('watchSystemTheme', () => {
  it('reports desktop changes and can be detached', () => {
    const media = stubMatchMedia(false)
    const seen: boolean[] = []
    const stop = watchSystemTheme((light) => seen.push(light))

    media.emit(true)
    media.emit(false)
    stop()
    media.emit(true)

    expect(seen).toEqual([true, false])
    expect(media.listenerCount).toBe(0)
  })

  it('is a no-op where matchMedia does not exist', () => {
    vi.stubGlobal('matchMedia', undefined)
    expect(() => watchSystemTheme(() => {})()).not.toThrow()
  })
})

describe('the stored preference', () => {
  beforeEach(() => {
    useSettings.getState().reset()
  })

  it('repaints the document as soon as the preference changes', () => {
    useSettings.getState().setTheme('light')
    expect(document.documentElement.dataset.theme).toBe('light')

    useSettings.getState().setTheme('dark')
    expect(document.documentElement.dataset.theme).toBe('dark')
  })

  it('keeps a value it does not recognise out of the store', () => {
    useSettings.getState().setTheme('midnight' as never)
    expect(useSettings.getState().theme).toBe(DEFAULT_THEME)
  })

  it('starts on system, so a fresh install follows the desktop', () => {
    expect(useSettings.getState().theme).toBe('system')
  })
})
