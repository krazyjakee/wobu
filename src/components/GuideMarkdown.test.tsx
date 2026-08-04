import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { GuideMarkdown } from './GuideMarkdown'
import { guideSlug, parseGuide } from './GuideMarkup'

/*
 * The guide's grammar is small and fixed, and the same files are also rendered
 * by `website/build.mjs` through a real Markdown library. So these tests are
 * about the two halves agreeing: the same source has to survive both, heading
 * ids have to match so a deep link works in either place, and a link has to
 * resolve to the right *kind* of destination — a page, an app surface, or the
 * system browser, which in a desktop webview are three different things.
 */

describe('the guide parser', () => {
  it('reads headings, paragraphs, lists, code and tables', () => {
    const blocks = parseGuide(
      [
        '# Title',
        '',
        'A paragraph that',
        'wraps across lines.',
        '',
        '## A section',
        '',
        '- one',
        '- two',
        '',
        '1. first',
        '2. second',
        '',
        '```',
        'code | not a table',
        '```',
        '',
        '| Head | Other |',
        '| --- | --- |',
        '| a | b |',
      ].join('\n'),
    )

    expect(blocks.map((b) => b.kind)).toEqual([
      'heading',
      'para',
      'heading',
      'list',
      'list',
      'code',
      'table',
    ])
    expect(blocks[1]).toMatchObject({ text: 'A paragraph that wraps across lines.' })
    expect(blocks[3]).toMatchObject({ ordered: false, items: ['one', 'two'] })
    expect(blocks[4]).toMatchObject({ ordered: true, items: ['first', 'second'] })
    expect(blocks[5]).toMatchObject({ text: 'code | not a table' })
    expect(blocks[6]).toMatchObject({ head: ['Head', 'Other'], rows: [['a', 'b']] })
  })

  it('turns a blockquote into a labelled callout', () => {
    const [block] = parseGuide('> **Bring your own key** Nothing is pre-configured.\n')
    expect(block).toMatchObject({
      kind: 'note',
      label: 'Bring your own key',
      paragraphs: ['Nothing is pre-configured.'],
    })
  })

  it('slugs a heading the way the website does', () => {
    expect(guideSlug('Notes & `Enhance`, top down')).toBe('notes-enhance-top-down')
  })

  it('keeps unrecognised text as prose rather than dropping it', () => {
    const blocks = parseGuide('~~~ not a fence ~~~\n')
    expect(blocks).toEqual([{ kind: 'para', text: '~~~ not a fence ~~~' }])
  })
})

describe('the guide renderer', () => {
  it('marks inline code, bold and emphasis', () => {
    render(<GuideMarkdown markdown="A `code` with **bold** and *slant*." />)
    expect(screen.getByText('code').tagName).toBe('CODE')
    expect(screen.getByText('bold').tagName).toBe('STRONG')
    expect(screen.getByText('slant').tagName).toBe('EM')
  })

  it('gives every heading the id its anchor expects', () => {
    const { container } = render(<GuideMarkdown markdown={'## How the stack resolves\n'} />)
    expect(container.querySelector('#how-the-stack-resolves')).not.toBeNull()
  })

  it('drops the empty header row a definition table has to be written with', () => {
    const table = ['| | |', '| --- | --- |', '| Mood | Never sent anywhere. |'].join('\n')
    const { container } = render(<GuideMarkdown markdown={table} />)
    expect(container.querySelector('thead')).toBeNull()
    expect(container.querySelectorAll('tbody tr')).toHaveLength(1)

    const named = ['| Role | Meaning |', '| --- | --- |', '| Mood | Yours. |'].join('\n')
    const withHead = render(<GuideMarkdown markdown={named} />)
    expect(withHead.container.querySelector('th')?.textContent).toBe('Role')
  })

  it('routes a sibling page, an app surface and a URL to three different handlers', () => {
    const onNavigate = vi.fn()
    const onAction = vi.fn()
    const onExternal = vi.fn()
    render(
      <GuideMarkdown
        markdown={
          'See [the stack](influence.md#layer-cards), [the keys](wobu:shortcuts) and ' +
          '[the repo](https://example.invalid/wobu).\n'
        }
        handlers={{ onNavigate, onAction, onExternal }}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'the stack' }))
    fireEvent.click(screen.getByRole('button', { name: 'the keys' }))
    fireEvent.click(screen.getByRole('button', { name: 'the repo' }))

    expect(onNavigate).toHaveBeenCalledWith('influence', 'layer-cards')
    expect(onAction).toHaveBeenCalledWith('shortcuts')
    expect(onExternal).toHaveBeenCalledWith('https://example.invalid/wobu')
  })

  it('offers a repository document as an external link, not as a missing page', () => {
    const onNavigate = vi.fn()
    const onExternal = vi.fn()
    render(
      <GuideMarkdown
        markdown={'The [data model](../02-data-model.md).'}
        handlers={{ onNavigate, onExternal }}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'data model' }))
    expect(onNavigate).not.toHaveBeenCalled()
    expect(onExternal).toHaveBeenCalledWith('../02-data-model.md')
  })

  it('renders nothing as an anchor, because a navigation would replace the app', () => {
    const { container } = render(
      <GuideMarkdown markdown={'[a page](workspace.md) and [a site](https://example.invalid/).'} />,
    )
    expect(container.querySelectorAll('a')).toHaveLength(0)
  })
})
