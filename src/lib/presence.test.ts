import { describe, expect, it } from 'vitest'
import type { Peer } from './api'
import {
  STALE_AFTER_SECS,
  editingText,
  editorsByNode,
  editorsOf,
  livePeers,
  openedText,
  sessionsText,
} from './presence'

/*
 * The rules behind four surfaces that all say a version of one sentence. What
 * is being pinned down here is mostly *restraint*: a session that stopped
 * beating has to disappear, an empty folder has to say nothing at all, and none
 * of it may turn into a reason to stop someone typing.
 */

function peer(over: Partial<Peer> & { sessionId: string }): Peer {
  return {
    user: over.sessionId,
    host: 'nas',
    seenSecsAgo: 0,
    editing: [],
    ...over,
  }
}

describe('a heartbeat that stopped', () => {
  it('is not shown as present once it is past the staleness bound', () => {
    // The laptop lid closed, or the VPN dropped. The backend reaps these when it
    // answers; this is the same bound applied to the answer we are holding,
    // because a poll that is failing keeps handing back the last one it got.
    const alive = peer({ sessionId: 'a', seenSecsAgo: 21 })
    const dead = peer({ sessionId: 'b', seenSecsAgo: STALE_AFTER_SECS + 1 })

    expect(livePeers([alive, dead])).toEqual([alive])
  })

  it('survives a missed beat or two, rather than flickering out and back', () => {
    // Sixty seconds is three beats for a reason: a share that stalls for a
    // moment is entirely ordinary, and a collaborator who blinks out of the UI
    // while sitting right there is how people learn to distrust the dot.
    expect(livePeers([peer({ sessionId: 'a', seenSecsAgo: 45 })])).toHaveLength(1)
    expect(livePeers([peer({ sessionId: 'a', seenSecsAgo: STALE_AFTER_SECS })])).toHaveLength(1)
  })

  it('takes its dot and its banner with it', () => {
    const dead = livePeers([peer({ sessionId: 'b', seenSecsAgo: 600, editing: ['kael'] })])
    expect(editorsByNode(dead).size).toBe(0)
    expect(editorsOf('kael', dead)).toEqual([])
  })

  it('treats no answer at all as nobody, not as an error', () => {
    expect(livePeers(undefined)).toEqual([])
    expect(openedText(livePeers(undefined))).toBeNull()
  })
})

describe('the greeting on open', () => {
  it('names one person the way the issue asks for', () => {
    expect(openedText([peer({ sessionId: 's1', user: 'Nadia' })])).toBe(
      'Nadia has this project open.',
    )
  })

  it('names two, and counts the rest after that', () => {
    const who = (n: number) =>
      ['Nadia', 'Tomas', 'Ilse', 'Bo']
        .slice(0, n)
        .map((user, i) => peer({ sessionId: `s${i}`, user }))

    expect(openedText(who(2))).toBe('Nadia and Tomas have this project open.')
    expect(openedText(who(3))).toBe('Nadia, Tomas and 1 other have this project open.')
    expect(openedText(who(4))).toBe('Nadia, Tomas and 2 others have this project open.')
  })

  it('says nothing when nobody else is here', () => {
    // The ordinary case. A greeting for an empty folder every single open is how
    // the one that matters gets dismissed unread.
    expect(openedText([])).toBeNull()
  })

  it('counts one person on two machines as one name', () => {
    // "Nadia and Nadia have this project open" reads as a rendering bug. The
    // session count in the status bar is where the second machine shows up.
    const twice = [
      peer({ sessionId: 's1', user: 'Nadia', host: 'desk' }),
      peer({ sessionId: 's2', user: 'Nadia', host: 'laptop' }),
    ]
    expect(openedText(twice)).toBe('Nadia has this project open.')
    expect(sessionsText(twice)).toBe('3 sessions')
  })
})

describe('who has which node open', () => {
  const nadia = peer({ sessionId: 's1', user: 'Nadia', editing: ['kael', 'ash'] })
  const tomas = peer({ sessionId: 's2', user: 'Tomas', editing: ['kael'] })

  it('indexes by node, so a row does not scan the peer list to say “nobody”', () => {
    const byNode = editorsByNode([nadia, tomas])
    expect(byNode.get('kael')).toBe('Nadia and Tomas')
    expect(byNode.get('ash')).toBe('Nadia')
    expect(byNode.get('a-node-nobody-opened')).toBeUndefined()
  })

  it('finds the peers on the node the editor is showing, and none on no node', () => {
    expect(editorsOf('ash', [nadia, tomas])).toEqual([nadia])
    expect(editorsOf(null, [nadia, tomas])).toEqual([])
  })
})

describe('the editor banner', () => {
  it('says what happens if you both save, because that is the only actionable part', () => {
    // The point of the sentence is that it is *not* a stop sign. If it did not
    // say what the collision costs, the only safe reading would be "wait", which
    // is the hard-lock behaviour advisory presence exists to avoid.
    const text = editingText([peer({ sessionId: 's1', user: 'Nadia' })], 'Kael')
    expect(text).toContain('Nadia has “Kael” open in another session')
    expect(text).toContain('Nothing is locked')
    expect(text).toContain('conflict')
  })
})

describe('the session count', () => {
  it('counts this session as well, because that is what the folder holds', () => {
    expect(sessionsText([peer({ sessionId: 's1' })])).toBe('2 sessions')
    expect(sessionsText([peer({ sessionId: 's1' }), peer({ sessionId: 's2' })])).toBe('3 sessions')
  })

  it('says nothing when this is the only session', () => {
    expect(sessionsText([])).toBeNull()
  })
})
