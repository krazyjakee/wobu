/**
 * The order and the filter behind every dropdown, kept out of the component.
 *
 * These are the two parts of a picker that go wrong quietly. A list that sorts
 * "Élan" after "Zephyr" because the accent sorts high, or that buries "The Ashen
 * Gate" under T, still *looks* sorted — nobody reports it, they just stop
 * trusting the order and scroll instead. So the rules live here, as pure
 * functions with their own tests, and `Combobox` only draws what they return.
 *
 * Every option is folded once per list rather than once per keystroke:
 * `prepareOptions` is memoised on the options, and typing only re-runs the
 * comparison, which is what keeps a few thousand rows responsive.
 */

/** The parts of an option this module reads; `Combobox` adds the rest. */
export interface OptionText {
  label: string
  /** Matched by the filter but never displayed — kinds, ids, alternate names. */
  keywords?: string
  /** Held at the head of the list by the sort — "All entities", "— top level —". */
  pinned?: boolean
}

export interface PreparedOption<T extends OptionText> {
  option: T
  /** Lowercased, accent-stripped label. */
  folded: string
  /** `folded` with a leading article removed — the key the sort uses. */
  title: string
  foldedKeywords: string
}

/*
 * English only, and deliberately so. The articles a world's titles start with
 * are the articles its author typed, and this application's own copy is
 * English; guessing at "le"/"la"/"der" would drop the first word of a Spanish
 * name like "La Muerte", which is part of the name rather than an article the
 * user wants ignored.
 */
const ARTICLES = ['the ', 'a ', 'an ']

/** Lowercase and strip diacritics, so "Élan" and "elan" compare as one string. */
export function foldText(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLocaleLowerCase()
}

/** `foldText` with a leading article removed: "The Ashen Gate" sorts under A. */
export function titleKey(label: string): string {
  const folded = foldText(label).trim()
  for (const article of ARTICLES) {
    if (folded.startsWith(article)) return folded.slice(article.length).trimStart()
  }
  return folded
}

export function prepareOptions<T extends OptionText>(options: T[]): PreparedOption<T>[] {
  return options.map((option) => ({
    option,
    folded: foldText(option.label),
    title: titleKey(option.label),
    foldedKeywords: option.keywords ? foldText(option.keywords) : '',
  }))
}

/**
 * Alphabetical by article-stripped, accent-folded title.
 *
 * Ties fall back to the folded label and then the raw one, so two options whose
 * sort keys collide — "The Gate" and "Gate" — still have one fixed order rather
 * than whichever order the caller's last fetch happened to produce. `sort` is
 * stable in every engine this ships on, so genuinely identical labels keep the
 * order they arrived in.
 *
 * Pinned rows are not sorted at all. "All entities" and "— top level —" are not
 * entries in the list, they are the way out of it, and alphabetising them into
 * the middle of the names would hide the row a user goes looking for first.
 */
export function sortPrepared<T extends OptionText>(
  prepared: PreparedOption<T>[],
): PreparedOption<T>[] {
  const pinned = prepared.filter((entry) => entry.option.pinned)
  const rest = prepared.filter((entry) => !entry.option.pinned)
  rest.sort(
    (a, b) =>
      a.title.localeCompare(b.title) ||
      a.folded.localeCompare(b.folded) ||
      a.option.label.localeCompare(b.option.label),
  )
  return [...pinned, ...rest]
}

/**
 * Rank of an option against an already-folded needle, or `-1` for no match.
 *
 * Three tiers, and no more: what the user typed starts the name (0), appears
 * somewhere in it (1), or only matches hidden keywords (2). A cleverer score —
 * fuzzy subsequences, word boundaries, recency — is a ranking nobody can
 * predict from the rows they can see, and an unpredictable order in a list you
 * are about to press Enter on is worse than a plain one.
 */
function matchRank<T extends OptionText>(entry: PreparedOption<T>, needle: string): number {
  if (entry.folded.startsWith(needle) || entry.title.startsWith(needle)) return 0
  if (entry.folded.includes(needle)) return 1
  if (entry.foldedKeywords.includes(needle)) return 2
  return -1
}

/** Matching options, prefix hits first, each tier in the order it came in. */
export function filterPrepared<T extends OptionText>(
  prepared: PreparedOption<T>[],
  query: string,
): PreparedOption<T>[] {
  const needle = foldText(query).trim()
  if (!needle) return prepared

  const scored: { entry: PreparedOption<T>; rank: number; index: number }[] = []
  for (const [index, entry] of prepared.entries()) {
    const rank = matchRank(entry, needle)
    if (rank >= 0) scored.push({ entry, rank, index })
  }
  // `index` as the tie-break rather than relying on sort stability alone: the
  // ranks are computed here, so the incoming order has to be carried explicitly
  // to survive being grouped by tier.
  scored.sort((a, b) => a.rank - b.rank || a.index - b.index)
  return scored.map((item) => item.entry)
}
