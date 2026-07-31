import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { App } from './App'
import { IconSprite } from './components/IconSprite'
import './styles/index.css'

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The file watcher pushes `world:changed`, so polling would be noise.
      refetchOnWindowFocus: false,
      refetchOnReconnect: false,
      retry: false,
      staleTime: 5_000,
    },
  },
})

const root = document.getElementById('root')
if (!root) throw new Error('#root missing from index.html')

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <IconSprite />
      <App />
    </QueryClientProvider>
  </StrictMode>,
)
