import { Fragment, type ReactNode } from 'react'
import { parseGuide } from './GuideMarkup'

/**
 * The parsed guide, as React elements.
 *
 * A link decides at render time whether it is another guide page, an app
 * surface to open, or a URL for the system browser — which is what makes the
 * guide navigable offline inside a desktop window, where following a real
 * `<a href>` would replace the running application.
 */

const INLINE = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\[[^\]]+\]\([^)\s]+\))|(\*[^*\s][^*]*\*)/g

export interface GuideLinkHandlers {
  /** Another page of the guide, by slug, optionally at a heading id. */
  onNavigate?: (slug: string, hash: string | null) => void
  /** An app surface named by a `wobu:` link — the guide pointing back inward. */
  onAction?: (action: string) => void
  /** Anything with a scheme. Opened outside the window, never inside it. */
  onExternal?: (href: string) => void
}

function GuideInlineLink({
  href,
  label,
  handlers,
}: {
  href: string
  label: string
  handlers: GuideLinkHandlers
}) {
  if (href.startsWith('wobu:')) {
    const action = href.slice('wobu:'.length)
    return (
      <button type="button" className="guide-action" onClick={() => handlers.onAction?.(action)}>
        {label}
      </button>
    )
  }

  if (/^[a-z]+:/i.test(href) || href.startsWith('//')) {
    return (
      <button type="button" className="guide-external" onClick={() => handlers.onExternal?.(href)}>
        {label}
      </button>
    )
  }

  const [path = '', hash = null] = href.split('#')
  // A sibling `*.md` is a page of this guide. Anything else is a repository
  // document that only exists on GitHub, so it is offered as an external link
  // rather than a route into a page that is not here.
  const slug = path.endsWith('.md') && !path.includes('/') ? path.slice(0, -3) : null
  if (!slug) {
    return (
      <button type="button" className="guide-external" onClick={() => handlers.onExternal?.(href)}>
        {label}
      </button>
    )
  }

  return (
    <button type="button" className="guide-xref" onClick={() => handlers.onNavigate?.(slug, hash)}>
      {label}
    </button>
  )
}

function guideInline(text: string, handlers: GuideLinkHandlers = {}): ReactNode {
  const out: ReactNode[] = []
  let last = 0
  let key = 0

  for (const match of text.matchAll(INLINE)) {
    const token = match[0]
    const at = match.index
    if (at > last) out.push(text.slice(last, at))
    last = at + token.length

    if (token.startsWith('`')) {
      out.push(<code key={key++}>{token.slice(1, -1)}</code>)
    } else if (token.startsWith('**')) {
      out.push(<strong key={key++}>{token.slice(2, -2)}</strong>)
    } else if (token.startsWith('[')) {
      const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(token)!
      out.push(<GuideInlineLink key={key++} href={link[2]!} label={link[1]!} handlers={handlers} />)
    } else {
      out.push(<em key={key++}>{token.slice(1, -1)}</em>)
    }
  }

  if (last < text.length) out.push(text.slice(last))
  return out.map((part, index) =>
    typeof part === 'string' ? <Fragment key={`t${index}`}>{part}</Fragment> : part,
  )
}

export function GuideMarkdown({
  markdown,
  handlers = {},
}: {
  markdown: string
  handlers?: GuideLinkHandlers
}) {
  const blocks = parseGuide(markdown)

  return (
    <>
      {blocks.map((block, index) => {
        const key = index
        switch (block.kind) {
          case 'heading': {
            const Tag = `h${block.depth}` as 'h1' | 'h2' | 'h3'
            return (
              <Tag key={key} id={block.id}>
                {guideInline(block.text, handlers)}
              </Tag>
            )
          }
          case 'para':
            return <p key={key}>{guideInline(block.text, handlers)}</p>
          case 'code':
            return (
              <pre key={key}>
                <code>{block.text}</code>
              </pre>
            )
          case 'list': {
            const items = block.items.map((item, n) => (
              <li key={n}>{guideInline(item, handlers)}</li>
            ))
            return block.ordered ? <ol key={key}>{items}</ol> : <ul key={key}>{items}</ul>
          }
          case 'table':
            return (
              <table key={key}>
                {/* A definition table — two columns, no column names — is
                    written with an empty header row, which GitHub-flavoured
                    Markdown requires and nobody should have to look at. */}
                {block.head.some((cell) => cell !== '') && (
                  <thead>
                    <tr>
                      {block.head.map((cell, n) => (
                        <th key={n}>{guideInline(cell, handlers)}</th>
                      ))}
                    </tr>
                  </thead>
                )}
                <tbody>
                  {block.rows.map((row, n) => (
                    <tr key={n}>
                      {row.map((cell, m) => (
                        <td key={m}>{guideInline(cell, handlers)}</td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          case 'note':
            return (
              <aside className="guide-note" key={key}>
                {block.label && <p className="guide-note-label">{block.label}</p>}
                {block.paragraphs.map((paragraph, n) => (
                  <p key={n}>{guideInline(paragraph, handlers)}</p>
                ))}
              </aside>
            )
        }
      })}
    </>
  )
}
