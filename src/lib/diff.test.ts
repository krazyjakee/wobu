import { describe, expect, it } from 'vitest'
import { MAX_CELLS, collapse, diffLines, hasChanges } from './diff'
import type { DiffRow } from './diff'

/** `-a` removed, `+b` added, `~a|b` changed, `=a` same, `…n` folded. */
const shape = (rows: DiffRow[]): string[] =>
  rows.map((r) => {
    switch (r.kind) {
      case 'same':
        return `=${r.left}`
      case 'removed':
        return `-${r.left}`
      case 'added':
        return `+${r.right}`
      case 'changed':
        return `~${r.left}|${r.right}`
      case 'gap':
        return `…${r.count}`
    }
  })

const lines = (...xs: string[]) => xs.join('\n')

describe('diffLines', () => {
  it('reports two identical documents as entirely unchanged', () => {
    // The commonest call by far: the card asks for a diff the moment it opens,
    // and "no visible difference" has to be a legible answer rather than noise.
    const rows = diffLines(lines('a', 'b', 'c'), lines('a', 'b', 'c'))
    expect(shape(rows)).toEqual(['=a', '=b', '=c'])
    expect(hasChanges(rows)).toBe(false)
  })

  it('pairs a rewritten line so the two versions sit on one row', () => {
    // Unpaired, a rewritten paragraph renders as N deletions then N additions
    // and the reader has to hold the first half in their head. Pairing is the
    // entire reason to lay a conflict out side by side.
    expect(shape(diffLines(lines('a', 'old', 'c'), lines('a', 'new', 'c')))).toEqual([
      '=a',
      '~old|new',
      '=c',
    ])
  })

  it('keeps the unchanged lines around an insertion', () => {
    expect(shape(diffLines(lines('a', 'c'), lines('a', 'b', 'c')))).toEqual(['=a', '+b', '=c'])
  })

  it('keeps the unchanged lines around a deletion', () => {
    expect(shape(diffLines(lines('a', 'b', 'c'), lines('a', 'c')))).toEqual(['=a', '-b', '=c'])
  })

  it('pairs what it can of an uneven rewrite and lists the rest', () => {
    // Two lines became three. The first two pair; the third has nothing on the
    // left, and dropping it — rather than showing it as an addition — would
    // hide a line of somebody's text from the person deciding.
    expect(shape(diffLines(lines('a', 'x', 'y', 'b'), lines('a', 'p', 'q', 'r', 'b')))).toEqual([
      '=a',
      '~x|p',
      '~y|q',
      '+r',
      '=b',
    ])
  })

  it('numbers both gutters against their own document', () => {
    // The numbers are how a user finds the line in their editor afterwards, so
    // an insertion has to push the right-hand numbers on without moving the
    // left-hand ones.
    const rows = diffLines(lines('a', 'c'), lines('a', 'b', 'c'))
    expect(rows.map((r) => [r.leftNo, r.rightNo])).toEqual([
      [1, 1],
      [null, 2],
      [2, 3],
    ])
  })

  it('treats an empty document as everything added', () => {
    expect(shape(diffLines('', lines('a', 'b')))).toEqual(['+a', '+b'])
    expect(shape(diffLines(lines('a', 'b'), ''))).toEqual(['-a', '-b'])
  })

  it('is empty for two empty documents', () => {
    expect(diffLines('', '')).toEqual([])
  })

  it('does not invent a blank last line from the trailing newline', () => {
    // Every file Wobu writes ends in a newline, so getting this wrong would put
    // a phantom changed row at the bottom of every single conflict card.
    expect(shape(diffLines('a\nb\n', 'a\nb\n'))).toEqual(['=a', '=b'])
    expect(shape(diffLines('a\nb\n', 'a\nb'))).toEqual(['=a', '=b'])
  })

  it('tells blank lines apart from missing ones', () => {
    // Markdown is whitespace-significant between blocks, so a removed blank
    // line is a real edit and has to show as one.
    expect(shape(diffLines(lines('a', '', 'b'), lines('a', 'b')))).toEqual(['=a', '-', '=b'])
  })

  it('finds a small change inside a long identical document', () => {
    // The head/tail trim is what keeps this cheap; the result has to be
    // identical to what the full table would have produced.
    const before = Array.from({ length: 400 }, (_, i) => `line ${i}`)
    const after = [...before]
    after[200] = 'line 200, rewritten'
    const rows = diffLines(before.join('\n'), after.join('\n'))

    expect(rows).toHaveLength(400)
    expect(rows.filter((r) => r.kind !== 'same')).toEqual([
      { kind: 'changed', left: 'line 200', right: 'line 200, rewritten', leftNo: 201, rightNo: 201 },
    ])
  })

  it('gives up on alignment rather than on answering, past the cell cap', () => {
    // Two large documents with nothing in common would need a table the webview
    // would stall building. The honest answer at that size is "these are
    // different", not a diff that freezes the window first.
    const side = Math.ceil(Math.sqrt(MAX_CELLS)) + 10
    const a = Array.from({ length: side }, (_, i) => `left ${i}`).join('\n')
    const b = Array.from({ length: side }, (_, i) => `right ${i}`).join('\n')

    const rows = diffLines(a, b)
    expect(rows).toHaveLength(side)
    expect(rows.every((r) => r.kind === 'changed')).toBe(true)
  })

  it('does not reorder lines to find a better match', () => {
    // Two paragraphs swapped is a real edit somebody made, and rendering it as
    // "unchanged" would hide a decision the card exists to ask about.
    const rows = diffLines(lines('a', 'b'), lines('b', 'a'))
    expect(hasChanges(rows)).toBe(true)
  })
})

describe('collapse', () => {
  const long = (n: number) => Array.from({ length: n }, (_, i) => `same ${i}`)

  it('folds a long identical run down to context on either side', () => {
    const before = [...long(20), 'old', ...long(20)].join('\n')
    const after = [...long(20), 'new', ...long(20)].join('\n')

    const rows = collapse(diffLines(before, after), 2)
    expect(shape(rows)).toEqual([
      '…18',
      '=same 18',
      '=same 19',
      '~old|new',
      '=same 0',
      '=same 1',
      '…18',
    ])
  })

  it('leaves a run short enough to be worth showing', () => {
    const rows = collapse(diffLines(lines('a', 'b', 'old', 'c', 'd'), lines('a', 'b', 'new', 'c', 'd')), 2)
    expect(shape(rows)).toEqual(['=a', '=b', '~old|new', '=c', '=d'])
  })

  it('folds a document with no changes at all into one gap', () => {
    // Nothing to anchor context to, so there is nothing worth showing. The card
    // says "no visible difference" instead.
    const rows = collapse(diffLines(long(30).join('\n'), long(30).join('\n')), 2)
    expect(shape(rows)).toEqual(['…30'])
  })

  it('counts every hidden line, so the gap can say how many', () => {
    const rows = collapse(diffLines(long(50).join('\n'), long(50).join('\n')), 3)
    const hidden = rows.filter((r) => r.kind === 'gap').reduce((n, r) => n + (r.count ?? 0), 0)
    expect(hidden).toBe(50)
  })

  it('changes nothing when everything differs', () => {
    const rows = diffLines(lines('a', 'b'), lines('c', 'd'))
    expect(collapse(rows, 2)).toEqual(rows)
  })

  it('is empty for an empty diff', () => {
    expect(collapse([], 2)).toEqual([])
  })
})
