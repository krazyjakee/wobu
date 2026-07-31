import { beforeEach, describe, expect, it } from 'vitest'
import { report, toast, useUI } from './ui'
import type { WobuError } from '../lib/api'

function err(over: Partial<WobuError> & { code: string }): WobuError {
  return { message: 'something went wrong', retryable: false, ...over }
}

beforeEach(() => {
  useUI.setState({ toasts: [], banners: [] })
})

describe('report — which surface an error lands on', () => {
  /*
   * The whole point of `report` is that call sites do not choose. If this
   * routing drifts, every future onError handler drifts with it, so the two
   * banner codes are pinned here explicitly rather than derived.
   */

  it('raises a banner for a share that went away', () => {
    report(err({ code: 'share.unmounted', message: 'the folder is not reachable', retryable: true }))
    const { banners, toasts } = useUI.getState()
    expect(toasts).toEqual([])
    expect(banners).toHaveLength(1)
    expect(banners[0]!.code).toBe('share.unmounted')
    expect(banners[0]!.retryable).toBe(true)
  })

  it('raises a banner for a folder that turned read-only mid-session', () => {
    report(err({ code: 'write.read_only' }))
    expect(useUI.getState().banners.map((b) => b.code)).toEqual(['write.read_only'])
  })

  it.each(['write.conflict', 'node.not_found', 'node.invalid', 'io.failed', 'internal'])(
    'toasts %s — the user knows which action caused it and the app still works',
    (code) => {
      report(err({ code }))
      expect(useUI.getState().banners).toEqual([])
      expect(useUI.getState().toasts).toHaveLength(1)
    },
  )

  it('toasts a plain Error, which never came from a command at all', () => {
    report(new Error('boom'))
    const { toasts } = useUI.getState()
    expect(toasts[0]!.kind).toBe('error')
    expect(toasts[0]!.text).toContain('boom')
  })

  it('prefixes the message so the toast says what was being attempted', () => {
    report(err({ code: 'io.failed', message: 'permission denied' }), 'Could not save')
    expect(useUI.getState().toasts[0]!.text).toBe('Could not save — permission denied')
  })

  it('carries the technical detail onto the banner', () => {
    report(err({ code: 'share.unmounted', detail: 'ENOENT: /Volumes/art' }))
    expect(useUI.getState().banners[0]!.detail).toBe('ENOENT: /Volumes/art')
  })
})

describe('banners', () => {
  it('collapses repeats of one code, keeping the newest wording', () => {
    // An unmounted share fails every read under it. Twenty identical banners is
    // a worse bug than none.
    const ui = useUI.getState()
    ui.raiseBanner({ code: 'share.unmounted', text: 'first', retryable: true })
    ui.raiseBanner({ code: 'share.unmounted', text: 'second', retryable: true })
    const { banners } = useUI.getState()
    expect(banners).toHaveLength(1)
    expect(banners[0]!.text).toBe('second')
  })

  it('keeps distinct codes side by side', () => {
    const ui = useUI.getState()
    ui.raiseBanner({ code: 'share.unmounted', text: 'a', retryable: true })
    ui.raiseBanner({ code: 'write.read_only', text: 'b', retryable: false })
    expect(useUI.getState().banners).toHaveLength(2)
  })

  it('clears one code without disturbing the others', () => {
    const ui = useUI.getState()
    ui.raiseBanner({ code: 'share.unmounted', text: 'a', retryable: true })
    ui.raiseBanner({ code: 'write.read_only', text: 'b', retryable: false })
    useUI.getState().clearBanner('share.unmounted')
    expect(useUI.getState().banners.map((b) => b.code)).toEqual(['write.read_only'])
  })

  it('clearing an absent code is a no-op, not a throw', () => {
    useUI.getState().clearBanner('nothing.here')
    expect(useUI.getState().banners).toEqual([])
  })
})

describe('toasts', () => {
  it('gives every toast a distinct id, so two identical messages both show', () => {
    toast('saved')
    toast('saved')
    const { toasts } = useUI.getState()
    expect(toasts).toHaveLength(2)
    expect(toasts[0]!.id).not.toBe(toasts[1]!.id)
  })

  it('drops by id', () => {
    toast('a')
    toast('b')
    const first = useUI.getState().toasts[0]!
    useUI.getState().dropToast(first.id)
    expect(useUI.getState().toasts.map((t) => t.text)).toEqual(['b'])
  })
})

describe('navigator width', () => {
  it('clamps to the usable range and rounds to whole pixels', () => {
    const { setNavWidth } = useUI.getState()
    setNavWidth(10)
    expect(useUI.getState().navWidth).toBe(200)
    setNavWidth(9999)
    expect(useUI.getState().navWidth).toBe(460)
    setNavWidth(272.6)
    expect(useUI.getState().navWidth).toBe(273)
  })
})

describe('openAncestors', () => {
  it('expands only the collapsed ones and leaves the rest alone', () => {
    useUI.setState({ collapsedNodes: { a: true, b: true, untouched: true } })
    useUI.getState().openAncestors(['a', 'b', 'never-collapsed'])
    expect(useUI.getState().collapsedNodes).toEqual({ untouched: true })
  })

  it('keeps the same object when nothing needed expanding', () => {
    // Identity matters: a fresh object here re-renders every row in the tree on
    // each selection change.
    const before = { x: true as const }
    useUI.setState({ collapsedNodes: before })
    useUI.getState().openAncestors(['y', 'z'])
    expect(useUI.getState().collapsedNodes).toBe(before)
  })
})
