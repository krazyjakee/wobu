const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'

/**
 * A fresh ULID, in the spelling `wobu_core::Id` parses.
 *
 * Minted here rather than asked for, because the command surface is coarse on
 * purpose: a scene is saved as a whole document, so there is no
 * `narrative_beat_add` to hand back an id, and adding one would be the first
 * step towards two implementations of every structural edit. The identity is
 * short-lived either way — the save returns the document the backend wrote, and
 * the canvas redraws from that.
 */
export function mintId(now = Date.now(), random = crypto.getRandomValues.bind(crypto)): string {
  let time = ''
  let remaining = now
  for (let index = 0; index < 10; index++) {
    time = CROCKFORD[remaining % 32] + time
    remaining = Math.floor(remaining / 32)
  }
  const bytes = random(new Uint8Array(16))
  let tail = ''
  for (const byte of bytes) tail += CROCKFORD[byte % 32]
  return time + tail
}
