import { beforeEach, describe, expect, it } from 'vitest'
import {
  GUIDE_GROUPS,
  GUIDE_PAGES,
  GUIDE_SOURCE_SLUGS,
  guidePage,
  openGuide,
  searchGuide,
  useGuide,
} from './GuideContent'
import { parseGuide } from './GuideMarkup'

/*
 * `docs/guide/` is read by two things that cannot see each other: this bundle
 * and `website/build.mjs`. Both walk it through `contents.json`, so a page
 * added to the directory and forgotten in the manifest would be published by
 * neither and noticed by nobody. These tests are that check, plus the one for
 * links between pages — a cross-reference to a page that does not exist is a
 * dead end in the app and a 404 on the site, and it is exactly the kind of
 * thing that rots quietly during a rename.
 */

beforeEach(() => {
  useGuide.setState({ slug: null, anchor: null })
})

describe('the guide corpus', () => {
  it('lists every page in the directory, and no page that is not there', () => {
    const listed = GUIDE_PAGES.map((page) => page.slug).sort()
    expect(listed).toEqual(GUIDE_SOURCE_SLUGS)
  })

  it('opens on the overview', () => {
    expect(GUIDE_PAGES[0]?.slug).toBe('index')
  })

  it('gives every page a title, a summary, a group and a heading of its own', () => {
    for (const page of GUIDE_PAGES) {
      expect(page.title.length, page.slug).toBeGreaterThan(0)
      expect(page.summary.length, page.slug).toBeGreaterThan(0)
      expect(page.group.length, page.slug).toBeGreaterThan(0)
      expect(parseGuide(page.markdown)[0], page.slug).toMatchObject({ kind: 'heading', depth: 1 })
      expect(page.sections.length, page.slug).toBeGreaterThan(0)
    }
  })

  it('resolves every cross-reference between pages', () => {
    const slugs = new Set(GUIDE_PAGES.map((page) => page.slug))
    for (const page of GUIDE_PAGES) {
      for (const [, href] of page.markdown.matchAll(/\]\(([^)\s]+)\)/g)) {
        const target = href!.split('#')[0] ?? ''
        if (!target.endsWith('.md') || target.includes('/')) continue
        expect(slugs, `${page.slug} links to ${target}`).toContain(target.slice(0, -3))
      }
    }
  })

  it('names only app surfaces the overlay knows how to open', () => {
    for (const page of GUIDE_PAGES) {
      for (const [, action] of page.markdown.matchAll(/\]\(wobu:([^)\s]+)\)/g)) {
        expect(['shortcuts', 'settings'], page.slug).toContain(action)
      }
    }
  })

  it('leaves no markup stranded in a paragraph', () => {
    for (const page of GUIDE_PAGES) {
      for (const block of parseGuide(page.markdown)) {
        if (block.kind !== 'para') continue
        // A pipe or a fence surviving into prose means a table or a code block
        // was written in a form the parser walked straight past.
        expect(block.text, `${page.slug}: ${block.text.slice(0, 60)}`).not.toMatch(/^\||```/)
      }
    }
  })

  it('groups the pages without losing or repeating one', () => {
    const grouped = GUIDE_GROUPS.flatMap((group) => group.pages)
    expect(grouped).toHaveLength(GUIDE_PAGES.length)
    expect(new Set(grouped.map((page) => page.slug)).size).toBe(GUIDE_PAGES.length)
  })

  it('finds a page by slug, and admits when there is none', () => {
    expect(guidePage('influence')?.title).toBe('The influence stack')
    expect(guidePage('nothing-like-this')).toBeUndefined()
  })
})

describe('searching the guide', () => {
  it('says nothing at all for a query too short to mean anything', () => {
    expect(searchGuide('a')).toEqual([])
  })

  it('finds a phrase from the middle of a page and names the section it is under', () => {
    const [hit] = searchGuide('Credential Manager')
    expect(hit?.page.slug).toBe('providers')
    expect(hit?.section?.text).toBe('How keys are stored')
    expect(hit?.excerpt.toLowerCase()).toContain('credential manager')
  })

  it('puts a title match above a body match', () => {
    const hits = searchGuide('generating')
    expect(hits[0]?.page.slug).toBe('generating')
    expect(hits[0]?.section).toBeNull()
  })
})

describe('where the guide is', () => {
  it('is closed until something opens it', () => {
    expect(useGuide.getState().slug).toBeNull()
  })

  it('opens on the overview by default, and at a heading when asked', () => {
    openGuide()
    expect(useGuide.getState()).toMatchObject({ slug: 'index', anchor: null })

    openGuide('influence', 'layer-cards')
    expect(useGuide.getState()).toMatchObject({ slug: 'influence', anchor: 'layer-cards' })

    useGuide.getState().close()
    expect(useGuide.getState()).toMatchObject({ slug: null, anchor: null })
  })
})
