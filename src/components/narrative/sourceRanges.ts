import { isAlias, isMap, isNode, isSeq, LineCounter, parseDocument } from 'yaml'

export interface SourceRange {
  start: number
  end: number
  line: number
  column: number
  alias?: string
}

/** CST offsets are UTF-16, the same units textarea selection uses. This index
 * never validates or prints source; Rust remains the single model authority. */
export function sourceRange(yaml: string): (path: (string | number)[]) => SourceRange | null {
  const lines = new LineCounter()
  const document = parseDocument(yaml, {
    lineCounter: lines,
    keepSourceTokens: true,
    strict: false,
  })
  return (path) => {
    let node: unknown = document.contents
    let nearest = isNode(node) ? node : null
    let consumedTag: unknown = null
    for (const part of path) {
      // Highlight the authored alias, rather than editing an anchor shared by
      // unrelated branches. Do not expand aliases or allocate their payloads.
      if (isAlias(node)) break
      if (isNode(node) && node !== consumedTag && node.tag === `!${part}`) {
        consumedTag = node
        continue
      }
      if (isMap(node) || isSeq(node)) node = node.get(part, true)
      else break
      if (isNode(node)) nearest = node
      else break
    }
    const range = nearest?.range
    if (!range) return null
    const position = lines.linePos(range[0])
    return {
      start: range[0],
      end: range[1],
      line: position.line,
      column: position.col,
      ...(isAlias(nearest) ? { alias: nearest.source } : {}),
    }
  }
}

/** Rust/libyaml columns count Unicode scalars, while textarea counts UTF-16. */
export function syntaxOffset(yaml: string, line: number, column: number): number {
  const rows = yaml.split('\n')
  const prefix = rows.slice(0, line - 1).reduce((sum, row) => sum + row.length + 1, 0)
  return Math.min(
    yaml.length,
    prefix +
      Array.from(rows[line - 1] ?? '')
        .slice(0, column - 1)
        .join('').length,
  )
}
