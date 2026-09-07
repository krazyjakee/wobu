import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST

function shouldBuildSourceMaps(mode: string, env: Record<string, string | undefined>) {
  return mode === 'diagnostic' || env.WOBU_BUILD_SOURCEMAPS === '1'
}

// Tauri expects devUrl http://localhost:1420 and frontendDist ../dist, so the
// port is fixed and the build output stays at the repo-root `dist` default.
export default defineConfig(({ mode }) => ({
  plugins: [react()],

  // Don't let Vite wipe Rust compiler errors off the screen.
  clearScreen: false,

  optimizeDeps: {
    /*
     * elkjs, named explicitly because it is only ever imported from inside a
     * Web Worker.
     *
     * Vite discovers dependencies by crawling the entry module, and the layout
     * worker is not on that path — so in dev the 1.6 MB GWT-compiled CommonJS
     * bundle is found late, on the first layout, and the optimizer restarts and
     * reloads the page underneath whoever asked for it. Listing it here has it
     * pre-bundled to ESM before the server starts serving.
     *
     * It stays in a worker, always: on the main thread elkjs costs +431 KB
     * gzipped and is parsed at start-up (docs/17-flow-canvas.md).
     */
    include: ['elkjs/lib/elk.bundled.js'],
  },

  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: {
      // src-tauri belongs to the Rust side; never trigger an HMR pass on it.
      ignored: ['**/src-tauri/**'],
    },
  },

  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'esnext',
    // Keep maps out of Tauri's packaged `frontendDist`. Diagnostic production
    // builds can opt in with `--mode diagnostic` or WOBU_BUILD_SOURCEMAPS=1.
    // @ts-expect-error process is a nodejs global
    sourcemap: shouldBuildSourceMaps(mode, process.env),
  },
}))
