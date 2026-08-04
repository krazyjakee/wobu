import { create } from 'zustand'
import contents from '../../docs/guide/contents.json'
import { guideSlug, parseGuide } from './GuideMarkup'

/**
 * The guide, compiled into the bundle.
 *
 * `docs/guide/*.md` is the single source: these files are what the reader sees
 * inside the app and what `website/build.mjs` publishes to wobu.app. They are
 * imported rather than fetched so the guide is present in the binary — Wobu is
 * local-first and the moment somebody most needs the guide is the moment their
 * network, their provider or their share has just failed.
 *
 * `contents.json` carries the running order and the titles, because a flat glob
 * has no opinion about which page a beginner should read second.
 */

interface ContentsPage {
  slug: string
  title: string
  summary: string
}

interface ContentsGroup {
  title: string
  pages: ContentsPage[]
}

export interface GuidePage extends ContentsPage {
  group: string
  markdown: string
  /** `## ` headings, for the in-page contents and for search. */
  sections: { id: string; text: string }[]
}

const sources = import.meta.glob('../../docs/guide/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

function markdownFor(slug: string): string {
  const source = sources[`../../docs/guide/${slug}.md`]
  if (source === undefined) throw new Error(`guide page ${slug}.md is listed but missing`)
  return source
}

export const GUIDE_GROUPS: { title: string; pages: GuidePage[] }[] = (
  contents.groups as ContentsGroup[]
).map((group) => ({
  title: group.title,
  pages: group.pages.map((page) => {
    const markdown = markdownFor(page.slug)
    return {
      ...page,
      group: group.title,
      markdown,
      sections: parseGuide(markdown).flatMap((block) =>
        block.kind === 'heading' && block.depth === 2 ? [{ id: block.id, text: block.text }] : [],
      ),
    }
  }),
}))

export const GUIDE_PAGES: GuidePage[] = GUIDE_GROUPS.flatMap((group) => group.pages)

/** Every `*.md` in the directory, so a test can prove none was left unlisted. */
export const GUIDE_SOURCE_SLUGS: string[] = Object.keys(sources)
  .map((path) => path.slice(path.lastIndexOf('/') + 1, -'.md'.length))
  .sort()

export function guidePage(slug: string): GuidePage | undefined {
  return GUIDE_PAGES.find((page) => page.slug === slug)
}

export interface GuideHit {
  page: GuidePage
  /** The heading the match sits under, when it is not the page itself. */
  section: { id: string; text: string } | null
  /** One line of the surrounding prose, for the result row. */
  excerpt: string
}

/**
 * Search the guide, plainly.
 *
 * Substring matching over the source text, ranked title-first. There is no
 * index and no stemming: a dozen documents is small enough that the honest
 * implementation is also the fast one, and a reader looking for "keychain"
 * types "keychain".
 */
export function searchGuide(query: string, pages: GuidePage[] = GUIDE_PAGES): GuideHit[] {
  const needle = query.trim().toLowerCase()
  if (needle.length < 2) return []

  const hits: GuideHit[] = []
  for (const page of pages) {
    const inTitle = page.title.toLowerCase().includes(needle)
    let section: GuideHit['section'] = null
    let excerpt = page.summary

    const lines = page.markdown.split('\n')
    let current: GuideHit['section'] = null
    let found = false
    for (const line of lines) {
      const heading = /^##\s+(.*)$/.exec(line)
      if (heading) {
        const text = heading[1]!.trim()
        current = { id: guideSlug(text), text }
      }
      if (found || !line.toLowerCase().includes(needle)) continue
      found = true
      section = current
      excerpt = line.replace(/^[#>\s|*-]+/, '').trim()
    }

    if (!inTitle && !found) continue
    hits.push({ page, section: inTitle ? null : section, excerpt })
  }

  return hits.sort((a, b) => {
    const aTitle = a.page.title.toLowerCase().includes(needle) ? 0 : 1
    const bTitle = b.page.title.toLowerCase().includes(needle) ? 0 : 1
    return aTitle - bTitle
  })
}

interface GuideState {
  /** The open page, or `null` when the guide is closed. */
  slug: string | null
  /** A heading to scroll to once the page is on screen. */
  anchor: string | null
  open: (slug?: string, anchor?: string | null) => void
  close: () => void
}

/**
 * Where the guide is, in one store.
 *
 * Kept beside the guide rather than in `store/ui.ts` because nothing else in
 * the app needs to read it: every surface that opens the guide does so through
 * `openGuide`, and the overlay is the only reader.
 */
export const useGuide = create<GuideState>((set) => ({
  slug: null,
  anchor: null,
  open: (slug = 'index', anchor = null) => set({ slug, anchor }),
  close: () => set({ slug: null, anchor: null }),
}))

/** Open the guide from anywhere, including outside React. */
export function openGuide(slug?: string, anchor?: string | null): void {
  useGuide.getState().open(slug, anchor)
}
