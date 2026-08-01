import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import prettier from 'eslint-config-prettier/flat'
import tseslint from 'typescript-eslint'

/**
 * Correctness only. Formatting is Prettier's job, and `eslint-config-prettier`
 * is extended last so nothing here can disagree with it — a rule that fires on
 * something `prettier --write` then reformats back is a loop, not a lint.
 *
 * Deliberately the non-type-checked `tseslint.configs.recommended`: `tsc
 * --noEmit` already runs in `npm run check` with `strict` plus
 * `noUncheckedIndexedAccess`, so a second type-aware pass would double the
 * cost of the local loop to re-report what the compiler has already said.
 */
export default tseslint.config(
  {
    ignores: ['dist/', 'src-tauri/target/'],
  },

  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat['recommended-latest'],
      reactRefresh.configs.vite,
      prettier,
    ],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    rules: {
      // A leading underscore is the repo's existing way of saying "bound on
      // purpose, not used" — destructuring a tuple to reach its second element,
      // mostly. tsc's `noUnusedLocals` honours the same convention.
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],

      // The three below ship as errors in `recommended-latest` and every one of
      // them already fires on code that works. They are demoted rather than
      // switched off, because each is worth reading before adding a new one —
      // but a lint gate that is red on arrival is a gate people learn to skip,
      // and this config exists to be adopted, not argued with.

      // Nine sites, all the same shape: an effect that clears derived state when
      // the thing it derives from changes. That is the documented React idiom
      // for it; the compiler-aware rule prefers a `key` or a render-time reset,
      // which is a refactor per call site rather than a lint fix.
      'react-hooks/set-state-in-effect': 'warn',
      // `useAutosaveNode` keeps a "latest value" ref so the debounce timer sends
      // what the user last typed rather than what was on screen when the timer
      // started. Reading and writing that ref during render is exactly what the
      // rule objects to and exactly what makes the hook correct.
      'react-hooks/refs': 'warn',
      // One site: `TitleBar.tsx` exports `modKey()` beside the component, which
      // costs that module a Fast Refresh partial update. Worth fixing when the
      // file is next opened, not worth blocking a push over.
      'react-refresh/only-export-components': 'warn',
    },
  },

  {
    // Build and test tooling runs under Node, not in the webview.
    files: ['*.config.{js,ts}', 'src/test/**/*.ts'],
    languageOptions: { globals: globals.node },
  },
)
