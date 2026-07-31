import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST

// Tauri expects devUrl http://localhost:1420 and frontendDist ../dist, so the
// port is fixed and the build output stays at the repo-root `dist` default.
export default defineConfig({
  plugins: [react()],

  // Don't let Vite wipe Rust compiler errors off the screen.
  clearScreen: false,

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
    sourcemap: true,
  },
})
