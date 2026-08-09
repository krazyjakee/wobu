import { describe, expect, it } from 'vitest'
import { errorMessage } from './api'
import type { WobuError } from './api'
import { humaniseBackendMessage, plainError } from './errorCopy'

function fail(over: Partial<WobuError> = {}): WobuError {
  return { code: 'internal', message: 'something', retryable: false, ...over }
}

describe('replacing a message that only made sense to the code', () => {
  it('does not put a ULID in front of somebody who deleted a character', () => {
    const text = plainError(
      fail({ code: 'node.not_found', message: 'no node with id 01ARZ3NDEKTSV4RRFFQ69G5FAV' }),
    )
    expect(text).not.toMatch(/01ARZ3ND/)
    expect(text).toMatch(/not in this project any more/)
  })

  it('answers a read-only folder with what to do about it', () => {
    const text = plainError(
      fail({ code: 'write.read_only', message: 'the project folder is read-only' }),
    )
    expect(text).toMatch(/permissions/)
  })

  it('says the same thing about a provider as the queue does', () => {
    // `failureCopy.ts` is the other half of this. One cause must not get one
    // answer from a job and a different one from a command.
    expect(plainError(fail({ code: 'provider.no_key' }))).toMatch(/Settings → Providers and models/)
  })

  it('does not turn a sync connection failure into disk-space advice', () => {
    const text = plainError(
      fail({ code: 'sync.unreachable', message: 'Could not reach the other machine.' }),
    )
    expect(text).toMatch(/other machine/)
    expect(text).toMatch(/online/)
    expect(text).not.toMatch(/disk space|project folder/)
  })

  it('rebuilds a conflict from its own field rather than the sentence', () => {
    const text = plainError(
      fail({
        code: 'write.conflict',
        message: 'Someone else changed this node while you were editing it.',
        conflictPath: 'nodes/species/vashk.conflict.md',
      }),
    )
    expect(text).toContain('nodes/species/vashk.conflict.md')
    expect(text).toContain('entity')
    expect(text).not.toContain('node while')
  })
})

describe('keeping a message that carries something only it knows', () => {
  it('adds guidance to a malformed file rather than throwing the path away', () => {
    const text = plainError(
      fail({ code: 'node.malformed', message: 'malformed node file nodes/species/vashk.md: bad' }),
    )
    expect(text).toContain('nodes/species/vashk.md')
    expect(text).toMatch(/text editor/)
  })

  it('leaves a code it has never heard of legible instead of silent', () => {
    expect(plainError(fail({ code: 'something.new', message: 'the sky fell in' }))).toBe(
      'The sky fell in.',
    )
  })
})

describe('humaniseBackendMessage', () => {
  it('says entity, because that is the word every label uses', () => {
    expect(humaniseBackendMessage('a node cannot be its own parent')).toBe(
      'An entity cannot be its own parent.',
    )
    expect(humaniseBackendMessage('unknown node kind: wyrm')).toBe('Unknown entity kind: wyrm.')
  })

  it('leaves the folder called nodes/ alone, because that is its name on disk', () => {
    expect(humaniseBackendMessage('nodes/species/vashk.md is missing its YAML frontmatter')).toBe(
      'Nodes/species/vashk.md is missing its YAML frontmatter.',
    )
  })

  it('finishes the sentence, since it is joined after a prefix and a dash', () => {
    expect(humaniseBackendMessage('cancelled')).toBe('Cancelled.')
    expect(humaniseBackendMessage('already done.')).toBe('Already done.')
  })
})

describe('the boundary itself', () => {
  it('is `errorMessage`, so no call site has to remember to ask', () => {
    expect(errorMessage(fail({ code: 'project.none_open', message: 'no project is open' }))).toBe(
      'No project is open.',
    )
  })

  it('leaves anything that never came from a command exactly as it was', () => {
    expect(errorMessage(new Error('boom'))).toBe('boom')
    expect(errorMessage('a bare string')).toBe('a bare string')
  })
})
