import { afterEach } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'

// React Testing Library only auto-cleans when Vitest globals are on, and they
// are not (see vitest.config.ts). Unmounting between tests matters more here
// than usual: useAutosaveNode flushes a pending save on unmount, so a component
// left mounted would fire its write into the *next* test's mock.
afterEach(cleanup)
