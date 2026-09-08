# Revision-aware localisation

Review → **Localisation** exports approved, current, locked dialogue and supporting text. Source
slots and wording variants keep their stable IDs. Translations have independent Draft/Approved
status and derived freshness; importing words never approves them. Review's **Translation readiness**
filter searches loaded source documents by locale, missing translation, draft, current approval or
out-of-date translation. The localisation dialog searches the complete captured catalog and renders
25 rows per page; import diagnostics render 50 per page.

## Interchange and decisions

Choose a target locale and CSV or JSON, then export to a new file outside the project. Existing
files are never overwritten. Each UTF-8 row carries format version, target locale, stable wording
ID and slot/container IDs, speaker, source text and revision, source guard, scene/context,
delivery notes, placeholder tokens, previous translation guard and plural forms. Choice labels use
their stable choice ID and a guard over their structural source and recorded dialogue unlocks;
their containing dialogue must be approved and locked before export. Only ready source is exported; the dialog still lists unready
source and missing translations so omissions are visible.

One file contains one target locale. JSON is an array of typed rows. CSV uses the exported column
order with RFC4180 quoting and CRLF row separators; embedded newlines, commas and quotes are
preserved. `forms` and `placeholders` are JSON cells. Imports are bounded to 16 MiB and 100,000 rows.
Edit only the forms cell; preserve source metadata and translation guards.

Load or paste the returned file and choose **Preview import**. Unknown, duplicate, missing, stale
source and placeholder diagnostics name the affected ID. A missing row leaves its existing
translation unchanged; a duplicate or invalid row is skipped. **Import eligible rows** rechecks
saved source and translation guards, publishes eligible rows individually, and reports partial
success and conflicts. It never silently refreshes an old translation guard. Resolve a conflict by
exporting current records and comparing both versions before submitting another import.

Inspect the displayed translation and its retained versions, then choose **Approve translation**.
The decision binds the exact forms and source guard. Source wording changes, relevant context
changes and unlocking source withdraw translation freshness. Relocking unchanged source does not
revive the previous translation: its guard includes the historical unlock event. Prior words and
approval decisions remain available for comparison. A source edit does not overwrite translated
words or turn an earlier Draft translation into Approved.

Canonical version-1 locale policy lives in a normal `narrative/policies/` envelope; each locale/ID
translation lives in `narrative/production/` and retains append-only decision history. Every published
translation head must match its immutable `narrative_locale_decision` receipt. Mutable approval flags
without that receipt do not verify. The normal guarded writer and conflict siblings handle concurrent
editors; receipts and production records use existing project sync. SQLite is not the locale authority.

Batch capture freezes source membership once, verifies each source/character observation and checks
the final whole-source fingerprint. Import reuses that capture and rechecks the selected source under
its editorial lock before each publication. Release additionally rechecks locale policy and verified
translation records after preparing the bundle. File watchers and review/source writes invalidate
both source and translation readiness reads.

## Locale, formatting and fallback contract

Locale IDs use `language[-Script][-REGION]`: a 2–3 letter language, optional four-letter script and
optional two-letter or three-digit region. Parsing accepts case differences and underscores, then
stores canonical case (`pt-BR`, `zh-Hant-HK`). Extensions, private-use and variant subtags are rejected.
The source locale defaults to `en` in the canonical policy.

Each configured Release locale explicitly either requires its own current approved translations or
permits fallback. Enabled fallback tries the requested locale, removes its final subtag repeatedly,
and finally tries source. It never substitutes a sibling region. Export reports every fallback and
every missing required translation. Development can export source-only content when required locale
rows are missing, with those diagnostics; its package retains the configured source locale and
omits target locales. Release blocks. Configuring no target locales preserves the existing
source-only package contract.

A placeholder is `{name}` or `{name:format}`; names start with an ASCII letter/underscore and may
continue with letters, digits, underscores or dots. A translation must preserve the complete token
set, including format suffixes. Order and repetition may change. `{{` and `}}` represent literal
braces. Other brace-containing prose is literal. Values are already formatted plain text supplied by
the host; this feature does not evaluate HTML, scripts or an ICU message grammar. Substituted values
are not parsed again.

Plural forms are explicit `zero`, `one`, `two`, `few`, `many`, `other` keys. `other` is mandatory and
is used when a selected category is absent. The host supplies the category; Wobu does not infer CLDR
categories from a number. Source-state variants remain available for authored grammatical branches.
The native reference graph uses `other`; `Bundle::render` provides explicit category and named-value
lookup. All forms preserve placeholders. Unicode is retained without normalization or reversal.
The editor uses `dir="auto"` and plain React text rendering for mixed-direction and RTL content.

## Native package and verification

Configured locale packages declare the optional `localisation: 1` capability and include a
hash-checked `locales.json` containing prepared strings and explicit fallback origins. The source
string table remains separate and its source locale appears in the manifest. Native package loading
validates locale identifiers, exact stable-ID coverage, forms, placeholders and permitted fallback;
`Package::graph_locale` resolves text without provider or filesystem access during execution.
Packages without locale configuration preserve their existing bytes and capabilities.

Rust tests cover CSV/JSON Unicode/quote/newline round trips, placeholders, plural fallback, missing
and duplicate IDs, mixed-locale refusal, real filesystem import, independent approval, retained
history after source unlock, stale/concurrent editors, receipt tampering, native package round-trip
and configured Release gates. Frontend tests exercise preview/import/approval, RTL rendering,
pagination, explicit fallback policy and project-switch protection. These are deterministic tests,
not live translation-provider quality checks.

For a fresh native fixture, from `src-tauri/` run:

```sh
cargo run -p wobu-store --example localisation_fixture -- /tmp/locale-fixture-parent
```

The parent directory must exist. This creates a new Harbor Voices project with all six types,
approved locked source, French Draft translations, approved Arabic translations, and strict French
and Arabic Release requirements. CSV files beside the project include quoted multiline text and
named placeholders. One line has handwritten French/Arabic example forms; remaining rows are
explicitly prefixed test text, not production translations. The native walkthrough can approve the
French rows, inspect Release, then unlock source and observe retained stale translations.

Unity, Godot, Unreal, Yarn and cross-language adapter conformance are excluded N5 work. The Rust
reference/package fixtures establish the native contract; they do not satisfy an engine-adapter-only
verification requirement. Native screenshots and observed walkthrough results belong in the
integration evidence record, separately from browser fixture tests.

Translation paragraphs use the locale's explicit script, or `Intl.Locale.maximize()` likely
script, for direction. Where available, the webview's locale text-info API supplies direction;
older webviews recognize Arab, Hebr, Thaa, Nkoo, Adlm, Rohg, Syrc, Samr and Mand as RTL, and Latn
as LTR. Unrecognized or incomplete locales fall back to automatic text direction. Explicit
`ar-Latn` therefore remains LTR, while a leading `{name}` cannot reverse Arabic. This follows the
[ECMA-402 LocaleDirection algorithm](https://tc39.es/proposal-intl-locale-info/#sec-Intl.Locale.prototype.getTextInfo)
and its [Unicode script metadata](https://unicode.org/reports/tr35/#Script_Metadata) reference.
CSS uses bidi isolation, preserving stored text order and the chosen paragraph direction;
`plaintext` would instead derive direction from the first strong character according to
[CSS Writing Modes](https://www.w3.org/TR/css-writing-modes-3/#valdef-unicode-bidi-plaintext).
