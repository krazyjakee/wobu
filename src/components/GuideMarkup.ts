/**
 * The guide's Markdown, parsed without a Markdown library.
 *
 * The guide is one authored corpus rather than arbitrary user input: it uses
 * headings, paragraphs, lists, tables, fenced blocks, blockquotes and four
 * inline marks, and it will keep using exactly those because the same files are
 * also published to the website by `website/build.mjs`. A parser for that
 * grammar is a hundred lines, and it buys something a library would not — the
 * result is a list of blocks rather than a string of HTML, so `GuideMarkdown`
 * can render React elements and no part of the app has to reach for
 * `dangerouslySetInnerHTML`.
 *
 * Anything the parser does not recognise falls through as a paragraph, so an
 * unsupported construct in a source file reads as plain prose rather than
 * disappearing.
 */

export type GuideBlock =
  | { kind: 'heading'; depth: 1 | 2 | 3; text: string; id: string }
  | { kind: 'para'; text: string }
  | { kind: 'list'; ordered: boolean; items: string[] }
  | { kind: 'code'; text: string }
  | { kind: 'table'; head: string[]; rows: string[][] }
  | { kind: 'note'; label: string | null; paragraphs: string[] }

/** Heading ids match `website/lib/markdown.mjs`, so anchors survive publishing. */
export function guideSlug(text: string): string {
  return text
    .toLowerCase()
    .replace(/`/g, '')
    .replace(/\*\*?/g, '')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/[^\w\- ]+/g, '')
    .trim()
    .replace(/\s+/g, '-')
}

function cells(row: string): string[] {
  return row
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim())
}

const DIVIDER = /^\|?[\s:|-]+\|[\s:|-]*$/

export function parseGuide(markdown: string): GuideBlock[] {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n')
  const blocks: GuideBlock[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]!

    if (line.trim() === '') {
      i += 1
      continue
    }

    if (line.startsWith('```')) {
      const body: string[] = []
      i += 1
      while (i < lines.length && !lines[i]!.startsWith('```')) {
        body.push(lines[i]!)
        i += 1
      }
      i += 1
      blocks.push({ kind: 'code', text: body.join('\n') })
      continue
    }

    const heading = /^(#{1,3})\s+(.*)$/.exec(line)
    if (heading) {
      const text = heading[2]!.trim()
      blocks.push({
        kind: 'heading',
        depth: heading[1]!.length as 1 | 2 | 3,
        text,
        id: guideSlug(text),
      })
      i += 1
      continue
    }

    if (line.startsWith('>')) {
      const quoted: string[] = []
      while (i < lines.length && lines[i]!.startsWith('>')) {
        quoted.push(lines[i]!.replace(/^>\s?/, ''))
        i += 1
      }
      const paragraphs = quoted
        .join('\n')
        .split(/\n\s*\n/)
        .map((p) => p.replace(/\n/g, ' ').trim())
        .filter(Boolean)
      const first = paragraphs[0] ?? ''
      const lead = /^\*\*([^*]+)\*\*\s*(?:—\s*)?/.exec(first)
      if (lead) {
        const rest = first.slice(lead[0].length).trim()
        paragraphs.splice(0, 1, ...(rest ? [rest] : []))
        blocks.push({ kind: 'note', label: lead[1]!.trim(), paragraphs })
      } else {
        blocks.push({ kind: 'note', label: null, paragraphs })
      }
      continue
    }

    if (line.trim().startsWith('|') && DIVIDER.test(lines[i + 1] ?? '')) {
      const head = cells(line)
      const rows: string[][] = []
      i += 2
      while (i < lines.length && lines[i]!.trim().startsWith('|')) {
        rows.push(cells(lines[i]!))
        i += 1
      }
      blocks.push({ kind: 'table', head, rows })
      continue
    }

    const bullet = /^(\s*)([-*]|\d+\.)\s+(.*)$/.exec(line)
    if (bullet) {
      const ordered = !/^[-*]$/.test(bullet[2]!)
      const items: string[] = []
      while (i < lines.length) {
        const next = /^(\s*)([-*]|\d+\.)\s+(.*)$/.exec(lines[i]!)
        if (next) {
          items.push(next[3]!.trim())
          i += 1
          continue
        }
        // A wrapped item: indented continuation of the bullet above it.
        if (/^\s{2,}\S/.test(lines[i]!) && items.length > 0) {
          items[items.length - 1] += ` ${lines[i]!.trim()}`
          i += 1
          continue
        }
        break
      }
      blocks.push({ kind: 'list', ordered, items })
      continue
    }

    const paragraph: string[] = []
    while (i < lines.length && lines[i]!.trim() !== '' && !/^(#|>|```|\s*[-*]\s)/.test(lines[i]!)) {
      paragraph.push(lines[i]!.trim())
      i += 1
    }
    blocks.push({ kind: 'para', text: paragraph.join(' ') })
  }

  return blocks
}
