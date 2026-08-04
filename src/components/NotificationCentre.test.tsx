import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { NotificationCentre } from './NotificationCentre'
import { reportJobFailure, useNotifications } from '../lib/notifications'
import type { JobFailure, JobSnapshot } from '../lib/api'
import { report, useUI } from '../store/ui'

function failed(over: Partial<JobFailure> = {}, job: Partial<JobSnapshot> = {}): JobSnapshot {
  return {
    id: 'j1',
    kind: 'generate',
    label: 'Generate Kael',
    subjectId: 'kael',
    attempt: 1,
    elapsedMs: 900,
    state: 'failed',
    retryHeld: false,
    failure: {
      code: 'internal',
      message:
        'this backend cannot honour the request: invalid_request: Image delivery mode is not supported.',
      retryable: false,
      billed: 'nothing',
      ...over,
    },
    ...job,
  } as JobSnapshot
}

function openCentre() {
  fireEvent.click(screen.getByRole('button', { name: /Notifications/ }))
}

beforeEach(() => {
  useNotifications.setState({ entries: [], open: false })
  useUI.setState({ toasts: [], banners: [], mode: 'library' })
})

describe('the notification centre', () => {
  it('counts what has not been read and explains the failure in words', () => {
    act(() => reportJobFailure(failed()))
    render(<NotificationCentre />)

    expect(screen.getByRole('button', { name: 'Notifications, 1 unread' })).toBeTruthy()
    openCentre()

    expect(screen.getByText('Image generation failed')).toBeTruthy()
    expect(screen.getByText(/fault in Wobu/)).toBeTruthy()
    // The provider's own sentence is present, but it is not the whole message.
    expect(screen.getByText(/cannot honour the request/)).toBeTruthy()
  })

  it('puts the amount charged in front of the user without a click', () => {
    act(() =>
      reportJobFailure(
        failed({ billed: 'charged', costNote: '1 mesh job' }, { kind: 'mesh', id: 'm1' }),
      ),
    )
    render(<NotificationCentre />)

    expect(
      screen.getByRole('button', { name: 'Notifications, 1 unread including a billed failure' }),
    ).toBeTruthy()
    openCentre()
    expect(
      screen.getByText(/charged for this attempt and got nothing back: 1 mesh job/),
    ).toBeTruthy()
  })

  it('keeps the technical remainder folded until it is asked for', () => {
    act(() => reportJobFailure(failed({ detail: 'HTTP 400 from generativelanguage' })))
    render(<NotificationCentre />)
    openCentre()

    expect(screen.queryByText('HTTP 400 from generativelanguage')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Details' }))
    expect(screen.getByText('HTTP 400 from generativelanguage')).toBeTruthy()
  })

  it('survives the component being unmounted and mounted again by a mode change', () => {
    act(() => reportJobFailure(failed()))
    const first = render(<NotificationCentre />)
    first.unmount()

    render(<NotificationCentre />)
    openCentre()
    expect(screen.getByText('Image generation failed')).toBeTruthy()
  })

  it('lets a failed generation be dismissed instead of sitting there for ever', () => {
    act(() => reportJobFailure(failed()))
    render(<NotificationCentre />)
    openCentre()

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss: Image generation failed' }))
    expect(screen.getByText(/Nothing to report/)).toBeTruthy()
  })

  it('clears the whole list on request and closes on Escape', () => {
    act(() => reportJobFailure(failed()))
    render(<NotificationCentre />)
    openCentre()

    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' })
    expect(screen.queryByRole('dialog')).toBeNull()

    openCentre()
    fireEvent.click(screen.getByRole('button', { name: 'Clear all' }))
    expect(screen.getByText(/Nothing to report/)).toBeTruthy()
  })

  it('captures a failed command that only ever had a vanishing toast', () => {
    render(<NotificationCentre />)
    act(() =>
      report({ code: 'io.failed', message: 'io error at /vol/art: no space', retryable: true }),
    )

    openCentre()
    // Not the crate's sentence: since #127 the boundary in `lib/errorCopy.ts`
    // says it in the app's own words. What is pinned here is that a failure
    // whose toast has gone is still readable, not which words it used.
    expect(screen.getByText(/could not read or write a file in the project folder/)).toBeTruthy()
  })
})
