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
    },
  },

  {
    // Build and test tooling runs under Node, not in the webview.
    files: ['*.config.{js,ts}', 'src/test/**/*.ts'],
    languageOptions: { globals: globals.node },
  },
)
