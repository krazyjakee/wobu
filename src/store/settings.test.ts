import { beforeEach, describe, expect, it } from 'vitest'
import {
  AUTOSAVE_DEFAULT,
  AUTOSAVE_MAX,
  AUTOSAVE_MIN,
  SCALE_MAX,
  SCALE_MIN,
  useSettings,
} from './settings'

beforeEach(() => {
  useSettings.getState().reset()
})

describe('clamping', () => {
  /*
   * These values come back from local storage, which is user-writable and
   * outlives any range this build believes in. An interface scaled to 0.05, or
   * an autosave debounce of zero writing on every keystroke over SMB, is an app
   * that cannot be recovered from inside the app.
   */

  it('holds the interface scale inside a usable range', () => {
    const { setUiScale } = useSettings.getState()
    setUiScale(0.05)
    expect(useSettings.getState().uiScale).toBe(SCALE_MIN)
    setUiScale(12)
    expect(useSettings.getState().uiScale).toBe(SCALE_MAX)
  })

  it('holds the autosave delay inside a usable range, and to whole milliseconds', () => {
    const { setAutosaveDelay } = useSettings.getState()
    setAutosaveDelay(0)
    expect(useSettings.getState().autosaveDelay).toBe(AUTOSAVE_MIN)
    setAutosaveDelay(60_000)
    expect(useSettings.getState().autosaveDelay).toBe(AUTOSAVE_MAX)
    setAutosaveDelay(432.7)
    expect(useSettings.getState().autosaveDelay).toBe(433)
  })

  it('accepts values inside the range unchanged', () => {
    useSettings.getState().setUiScale(1.2)
    useSettings.getState().setAutosaveDelay(1500)
    expect(useSettings.getState().uiScale).toBe(1.2)
    expect(useSettings.getState().autosaveDelay).toBe(1500)
  })
})

describe('defaults', () => {
  it('starts at exactly one, so no zoom is applied at all', () => {
    // `useUiScale` treats 1 as "remove the property" rather than "zoom: 1",
    // which only works if the default really is 1 and not 0.999…
    expect(useSettings.getState().uiScale).toBe(1)
  })

  it('reset puts everything back', () => {
    useSettings.getState().setUiScale(1.4)
    useSettings.getState().setAutosaveDelay(2000)
    useSettings.getState().reset()
    expect(useSettings.getState()).toMatchObject({
      uiScale: 1,
      autosaveDelay: AUTOSAVE_DEFAULT,
    })
  })
})

describe('rehydration', () => {
  it('clamps what it reads back, not just what it is given', () => {
    // A stored value written before a range changed, or edited by hand. persist
    // merges the stored object over the defaults, so without a merge that
    // clamps, an out-of-range number survives every restart.
    localStorage.setItem(
      'wobu.settings',
      JSON.stringify({ state: { uiScale: 9, autosaveDelay: -5 }, version: 0 }),
    )
    useSettings.persist.rehydrate()

    expect(useSettings.getState().uiScale).toBe(SCALE_MAX)
    expect(useSettings.getState().autosaveDelay).toBe(AUTOSAVE_MIN)
    localStorage.removeItem('wobu.settings')
  })

  it('falls back to the defaults for a stored blob missing keys', () => {
    localStorage.setItem('wobu.settings', JSON.stringify({ state: {}, version: 0 }))
    useSettings.persist.rehydrate()

    expect(useSettings.getState().uiScale).toBe(1)
    expect(useSettings.getState().autosaveDelay).toBe(AUTOSAVE_DEFAULT)
    localStorage.removeItem('wobu.settings')
  })

  it('survives a stored blob that is not what this build expects', () => {
    localStorage.setItem('wobu.settings', JSON.stringify({ state: { uiScale: 'big' }, version: 0 }))
    useSettings.persist.rehydrate()

    // `Math.min(hi, Math.max(lo, NaN))` is NaN, not a clamp, so without an
    // explicit finite check this lands on the document root as `zoom: NaN`.
    expect(useSettings.getState().uiScale).toBe(1)
    localStorage.removeItem('wobu.settings')
  })
})
