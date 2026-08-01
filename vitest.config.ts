import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

/**
 * Deliberately separate from `vite.config.ts`.
 *
 * That file is Tauri's — it pins port 1420 and `strictPort`, so merging the two
 * would mean a test run and `npm run tauri dev` fighting over a socket. Vitest
 * prefers this file when both exist, and the app build never loads it.
 */
export default defineConfig({
  plugins: [react()],
  test: {
    // jsdom, not happy-dom: the autosave hook uses window.setTimeout/clearTimeout
    // and document visibility, and jsdom is the one that matches the browser.
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    // The UI suites are jsdom-heavy. Bounding concurrency avoids starving
    // React Query notifications and turning one-second DOM waits into flakes.
    maxWorkers: 4,
    // `globals: false` — every `describe`/`it`/`expect` is imported. It keeps
    // the test files honest about what they depend on and means tsc type-checks
    // them with no extra `types` entry in tsconfig.json.
    globals: false,
  },
})
