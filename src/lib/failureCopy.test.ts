import { describe, expect, it } from 'vitest'
import type { JobFailure } from './api'
import { chargeLine, failureGuidance, failureNotice } from './failureCopy'

function failure(over: Partial<JobFailure> = {}): JobFailure {
  return {
    code: 'provider.unavailable',
    message: 'the backend did not answer',
    retryable: true,
    billed: 'nothing',
    ...over,
  }
}

describe('translating a provider rejection', () => {
  // The failure #142 was filed about, verbatim.
  const delivery = failure({
    code: 'internal',
    message:
      'this backend cannot honour the request: invalid_request: Image delivery mode is not supported.',
    retryable: false,
  })

  it('answers a request the backend refused with something the user can act on', () => {
    const guidance = failureGuidance(delivery)

    expect(guidance).toMatch(/fault in Wobu/)
    expect(guidance).toMatch(/update/i)
    expect(guidance).not.toMatch(/prompt is wrong/)
  })

  it('keeps the backend’s own sentence as evidence rather than as the whole message', () => {
    const notice = failureNotice('generate', delivery)

    expect(notice.title).toBe('Image generation failed')
    expect(notice.reason).toBe(delivery.message)
    expect(notice.guidance).not.toBe(delivery.message)
  })
})

describe('a failure that cost money', () => {
  it('says what was billed, in the title as well as the body', () => {
    const notice = failureNotice('mesh', failure({ billed: 'charged', costNote: '1 mesh job' }))

    expect(notice.title).toBe('3D reconstruction failed after it was billed')
    expect(notice.charge).toBe(
      'You were charged for this attempt and got nothing back: 1 mesh job.',
    )
  })

  it('does not round an unknown charge down to free', () => {
    expect(chargeLine(failure({ billed: 'unknown' }))).toMatch(/did not say whether/)
    expect(chargeLine(failure({ billed: 'nothing' }))).toBeNull()
  })
})

describe('guidance per code', () => {
  it('names the whole route to the fix rather than saying "check your settings"', () => {
    expect(failureGuidance(failure({ code: 'provider.no_key' }))).toMatch(
      /Settings → Providers and add one/,
    )
  })

  it('repeats the backend’s own wait hint when it gave one', () => {
    const limited = failure({ code: 'provider.rate_limited', retryAfter: 30_000 })
    expect(failureGuidance(limited)).toMatch(/30 seconds/)
    expect(failureGuidance(failure({ code: 'provider.rate_limited' }))).toMatch(
      /did not say how long/,
    )
  })

  it('never leaves a user with no next step, even for a code it has never heard of', () => {
    expect(failureGuidance(failure({ code: 'provider.brand_new_thing' }))).not.toBe('')
    expect(
      failureGuidance(failure({ code: 'provider.brand_new_thing', retryable: false })),
    ).toMatch(/would fail the same way/)
  })
})
