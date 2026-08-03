import { beforeEach, describe, expect, it } from 'vitest'
import type { JobFailure, JobSnapshot } from './api'
import {
  reportJobFailure,
  subscribeErrorToasts,
  unreadCostsMoney,
  useNotifications,
} from './notifications'
import { report, toast, useUI } from '../store/ui'

function failed(over: Partial<JobFailure> = {}, job: Partial<JobSnapshot> = {}): JobSnapshot {
  const failure: JobFailure = {
    code: 'provider.unavailable',
    message: 'connection refused on 127.0.0.1:8188',
    retryable: true,
    billed: 'nothing',
    ...over,
  }
  return {
    id: 'j1',
    kind: 'generate',
    label: 'Generate Kael',
    subjectId: 'kael',
    attempt: 1,
    elapsedMs: 1_200,
    state: 'failed',
    retryHeld: false,
    failure,
    ...job,
  } as JobSnapshot
}

beforeEach(() => {
  useNotifications.setState({ entries: [], open: false })
  useUI.setState({ toasts: [], banners: [], mode: 'library' })
})

describe('surfacing a job failure', () => {
  it('records it and announces it once, however many snapshots repeat it', () => {
    const job = failed()
    reportJobFailure(job)
    reportJobFailure(job)
    reportJobFailure(job)

    expect(useNotifications.getState().entries).toHaveLength(1)
    expect(useUI.getState().toasts).toHaveLength(1)
  })

  it('treats a fresh attempt of the same job as a new failure', () => {
    reportJobFailure(failed())
    reportJobFailure(failed({}, { attempt: 2 }))

    expect(useNotifications.getState().entries).toHaveLength(2)
  })

  it('says nothing about a job the user cancelled', () => {
    reportJobFailure(failed({ code: 'cancelled', message: 'cancelled' }))

    expect(useNotifications.getState().entries).toHaveLength(0)
    expect(useUI.getState().toasts).toHaveLength(0)
  })

  it('ignores a job that has not failed', () => {
    reportJobFailure({
      id: 'j2',
      kind: 'generate',
      label: 'Generate Kael',
      subjectId: 'kael',
      attempt: 1,
      elapsedMs: 10,
      state: 'running',
    })

    expect(useNotifications.getState().entries).toHaveLength(0)
  })
})

describe('a billed failure', () => {
  const billed = failed(
    { code: 'provider.bad_response', billed: 'charged', costNote: '1 mesh job' },
    { kind: 'mesh', label: 'Reconstruct Kael' },
  )

  it('states the cost in the toast itself rather than behind a disclosure', () => {
    reportJobFailure(billed)

    const [announced] = useUI.getState().toasts
    expect(announced?.text).toContain('1 mesh job')
    expect(announced?.kind).toBe('error')
    // Money must not expire while the user is in another window.
    expect(announced?.persistent).toBe(true)
  })

  it('marks the unread badge as owed so it is visible without opening anything', () => {
    reportJobFailure(billed)
    expect(unreadCostsMoney(useNotifications.getState().entries)).toBe(true)

    useNotifications.getState().clear()
    reportJobFailure(failed({}, { id: 'j9' }))
    expect(unreadCostsMoney(useNotifications.getState().entries)).toBe(false)
  })
})

describe('the action a failure offers', () => {
  it('sends an unconfigured backend to the place the key is entered', () => {
    reportJobFailure(failed({ code: 'provider.no_key', retryable: false }))

    const [entry] = useNotifications.getState().entries
    expect(entry?.action?.label).toBe('Open Settings')
    entry?.action?.run()
    expect(useUI.getState().mode).toBe('settings')
    expect(useNotifications.getState().open).toBe(false)
  })

  it('sends everything else to the entity whose generation failed', () => {
    reportJobFailure(failed())

    const [entry] = useNotifications.getState().entries
    entry?.action?.run()
    expect(useUI.getState().selectedId).toBe('kael')
    expect(useUI.getState().tab).toBe('concepts')
  })

  it('offers nothing to navigate to when the job had no subject', () => {
    reportJobFailure(failed({}, { subjectId: null }))

    expect(useNotifications.getState().entries[0]?.action).toBeUndefined()
  })
})

describe('mirroring the toasts every failed command already raises', () => {
  it('keeps a command failure readable after its toast has gone', () => {
    const stop = subscribeErrorToasts()
    report({ code: 'write.conflict', message: 'Saved elsewhere first', retryable: false })
    stop()

    const entries = useNotifications.getState().entries
    expect(entries).toHaveLength(1)
    expect(entries[0]?.title).toContain('Saved elsewhere first')
  })

  it('leaves confirmations alone', () => {
    const stop = subscribeErrorToasts()
    toast('Concept deleted')
    stop()

    expect(useNotifications.getState().entries).toHaveLength(0)
  })

  it('does not file a job failure twice when both paths are live', () => {
    const stop = subscribeErrorToasts()
    reportJobFailure(failed())
    // The job's own toast is still on screen when an unrelated one arrives. A
    // mirror that re-read the whole list here would file the failure again.
    toast('Concept deleted')
    report({ code: 'io.failed', message: 'Could not write the file', retryable: true })
    stop()

    const entries = useNotifications.getState().entries
    expect(entries).toHaveLength(2)
    expect(entries.filter((entry) => entry.title.startsWith('Image generation'))).toHaveLength(1)
  })
})
