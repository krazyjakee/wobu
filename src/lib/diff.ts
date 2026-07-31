/**
 * A line-level diff of two Markdown documents, laid out side by side.
 *
 * Written here rather than pulled in, and that is a deliberate trade. This is
 * an LCS over a few hundred lines — the algorithm below is textbook and fits on
 * a screen — while a diff dependency arrives with a transitive tree, a release
 * cadence and a supply chain, all of it running against the one class of file
 * in Wobu that two people can lose work in. The cost of owning fifty lines is
 * lower than the cost of trusting somebody else's for this.
 *
 * Nothing here merges. The rows describe what differs so a person can decide;
 * there is no code path that produces a third document from the two, because
 * three sentences two people rewrote in different directions have no correct
 * interleaving and a machine guessing at one writes text neither of them wrote.
 * See `docs/07-file-shares.md`.
 */

export interface DiffRow {
  /** `gap` is a run of identical lines folded away; it carries only `count`. */
  kind: 'same' | 'changed' | 'added' | 'removed' | 'gap'
  left: string | null
  right: string | null
  /** 1-based, for the gutter. Null on the side that has no line here. */
  leftNo: number | null
  rightNo: number | null
  /** Only on `gap`: how many identical lines are hidden. */
  count?: number
}

/**
 * Above this many cells the LCS table is abandoned and the two documents are
 * reported as wholly different.
 *
 * The table is `left × right` 32-bit cells, so the limit is really a memory
 * cap: two thousand lines against two thousand is 16MB and a visible pause in
 * the webview. A node file that large is not something anyone reads a
 * line-by-line diff of anyway — the honest answer at that size is "these are
 * different, open both" rather than a diff that freezes the window first.
 */
export const MAX_CELLS = 4_000_000

/** One trailing newline is the file ending, not an empty last line. */
function toLines(text: string): string[] {
  if (!text) return []
  return text.replace(/\n$/, '').split('\n')
}

interface Op {
  kind: 'same' | 'removed' | 'added'
  a: number
  b: number
}

/**
 * The longest common subsequence of two line arrays, as a list of operations.
 *
 * Filled backwards so the walk forwards can be greedy, which is what makes the
 * output stable: given a choice between deleting and inserting, it always takes
 * the same branch, so the same pair of files always produces the same rows.
 */
function lcsOps(a: string[], b: string[]): Op[] {
  const n = a.length
  const m = b.length
  const w = m + 1
  const table = new Int32Array((n + 1) * w)

  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      table[i * w + j] =
        a[i] === b[j]
          ? table[(i + 1) * w + j + 1]! + 1
          : Math.max(table[(i + 1) * w + j]!, table[i * w + j + 1]!)
    }
  }

  const ops: Op[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (a[i] === b[j]) ops.push({ kind: 'same', a: i++, b: j++ })
    else if (table[(i + 1) * w + j]! >= table[i * w + j + 1]!) ops.push({ kind: 'removed', a: i++, b: j })
    else ops.push({ kind: 'added', a: i, b: j++ })
  }
  while (i < n) ops.push({ kind: 'removed', a: i++, b: j })
  while (j < m) ops.push({ kind: 'added', a: i, b: j++ })
  return ops
}

function sameRow(line: string, leftNo: number, rightNo: number): DiffRow {
  return { kind: 'same', left: line, right: line, leftNo, rightNo }
}

/**
 * Pair a run of removals with the run of additions that follows it.
 *
 * Without this a rewritten paragraph renders as five deleted rows and then five
 * added rows, and the reader has to hold the first half in their head to
 * compare it with the second. Paired, the old and the new sit on one line,
 * which is the entire reason to lay a conflict out side by side.
 */
function pair(removed: Op[], added: Op[], a: string[], b: string[], offset: number): DiffRow[] {
  const rows: DiffRow[] = []
  for (let k = 0; k < Math.max(removed.length, added.length); k++) {
    const r = removed[k]
    const d = added[k]
    if (r && d) {
      rows.push({
        kind: 'changed',
        left: a[r.a]!,
        right: b[d.b]!,
        leftNo: offset + r.a + 1,
        rightNo: offset + d.b + 1,
      })
    } else if (r) {
      rows.push({ kind: 'removed', left: a[r.a]!, right: null, leftNo: offset + r.a + 1, rightNo: null })
    } else if (d) {
      rows.push({ kind: 'added', left: null, right: b[d.b]!, leftNo: null, rightNo: offset + d.b + 1 })
    }
  }
  return rows
}

/**
 * Side-by-side rows for two documents. `left` is conventionally the version on
 * disk and `right` the parked one, but nothing here depends on which is which.
 */
export function diffLines(left: string, right: string): DiffRow[] {
  const a = toLines(left)
  const b = toLines(right)

  // Trim the identical head and tail before doing any work. A conflict is
  // almost always one edited paragraph inside a file that is otherwise
  // character-identical, so this alone takes the usual case from a table of
  // hundreds of thousands of cells to one of a few dozen.
  let head = 0
  while (head < a.length && head < b.length && a[head] === b[head]) head++
  let tail = 0
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) {
    tail++
  }

  const midA = a.slice(head, a.length - tail)
  const midB = b.slice(head, b.length - tail)

  const rows: DiffRow[] = []
  for (let i = 0; i < head; i++) rows.push(sameRow(a[i]!, i + 1, i + 1))

  if (midA.length * midB.length > MAX_CELLS) {
    // Too big to align honestly, so it is not aligned at all rather than
    // aligned badly: everything on the left is removed, everything on the
    // right added, and the card's "open both" is the real answer.
    rows.push(...pair(midA.map((_, k) => ({ kind: 'removed' as const, a: k, b: 0 })),
                      midB.map((_, k) => ({ kind: 'added' as const, a: 0, b: k })),
                      midA, midB, head))
  } else {
    let k = 0
    const ops = lcsOps(midA, midB)
    while (k < ops.length) {
      const op = ops[k]!
      if (op.kind === 'same') {
        rows.push(sameRow(midA[op.a]!, head + op.a + 1, head + op.b + 1))
        k++
        continue
      }
      // One maximal run of changes, however the ops happen to interleave
      // removals and additions inside it.
      const removed: Op[] = []
      const added: Op[] = []
      while (k < ops.length && ops[k]!.kind !== 'same') {
        const next = ops[k]!
        if (next.kind === 'removed') removed.push(next)
        else added.push(next)
        k++
      }
      rows.push(...pair(removed, added, midA, midB, head))
    }
  }

  for (let i = 0; i < tail; i++) {
    const ai = a.length - tail + i
    const bi = b.length - tail + i
    rows.push(sameRow(a[ai]!, ai + 1, bi + 1))
  }
  return rows
}

export function hasChanges(rows: DiffRow[]): boolean {
  return rows.some((r) => r.kind !== 'same')
}

/**
 * Fold long runs of identical lines into a single `gap` row.
 *
 * A conflict on a long node file is a handful of changed lines inside hundreds
 * that match, and rendering all of them buries the decision the card is asking
 * for. `context` lines survive on each side of every change so the reader can
 * see where they are; the card's "open both" turns this off entirely.
 */
export function collapse(rows: DiffRow[], context = 3): DiffRow[] {
  const out: DiffRow[] = []
  let i = 0
  while (i < rows.length) {
    if (rows[i]!.kind !== 'same') {
      out.push(rows[i]!)
      i++
      continue
    }
    const start = i
    while (i < rows.length && rows[i]!.kind === 'same') i++
    const run = rows.slice(start, i)
    // A run bounded by a change on both sides keeps context twice over; one at
    // the start or end of the document only needs it on the inside.
    const keepHead = start === 0 ? 0 : context
    const keepTail = i === rows.length ? 0 : context
    if (run.length <= keepHead + keepTail + 1) {
      out.push(...run)
      continue
    }
    out.push(...run.slice(0, keepHead))
    out.push({
      kind: 'gap',
      left: null,
      right: null,
      leftNo: null,
      rightNo: null,
      count: run.length - keepHead - keepTail,
    })
    out.push(...run.slice(run.length - keepTail))
  }
  return out
}
