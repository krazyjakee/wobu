import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
  type UseQueryResult,
} from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import * as api from '../api'
import type { Capability, KeyStatus, ProviderSelections, QueueSnapshot } from '../api'
import { report } from '../../store/ui'
import { qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/**
 * Whether this machine has a key for each of these providers.
 *
 * Presence, never value: there is no query here that returns key material,
 * because there is no command that returns it. What this answers is the
 * question the UI actually has — "Gemini is selected; can this machine run it?"
 *
 * `staleTime: Infinity` because the answer only changes when this app changes
 * it, and the two mutations below invalidate when they do. A key edited in
 * Seahorse or Keychain Access while Wobu is open is picked up on the next run;
 * the Rust side caches for the same reason, which is that a locked Secret
 * Service prompts the user on every read.
 */
export function useProviderKeys(providers: string[]): UseQueryResult<KeyStatus[]> {
  return useQuery({
    queryKey: qk.providerKeys(providers),
    queryFn: () => api.providerKeyStatus(providers),
    staleTime: Infinity,
    retry: false,
  })
}

/**
 * Save a key for a provider.
 *
 * Deliberately **not** a `useMutation`, and that is the whole point of the hook.
 * React Query keeps `variables` on a mutation until something resets it, so a
 * key passed through one stays reachable from the mutation cache — and the
 * mutation cache is a long-lived object hanging off the `QueryClient`, which
 * means a pasted key would outlive the form, the pane, and every re-render, in
 * the one process the rest of this design works to keep key material out of.
 * `keys.rs` guarantees nothing sends a key back to the webview; that guarantee
 * is worth very little if the webview keeps its own copy.
 *
 * So the key is an argument to a plain call and a local for the length of one
 * await. The caller passes it straight from the DOM node the user typed it into
 * and clears that node afterwards — see `Settings.tsx`, which never puts one in
 * React state either.
 *
 * `saving` is the provider id, not a boolean: the pane renders every provider at
 * once and a shared flag would put "Saving…" on all of them.
 */
export function useSetProviderKey() {
  const qc = useQueryClient()
  const [saving, setSaving] = useState<string | null>(null)

  const save = useCallback(
    async (provider: string, key: string) => {
      setSaving(provider)
      try {
        await api.providerKeySet(provider, key)
        void qc.invalidateQueries({ queryKey: ['provider_key_status'] })
        void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
      } finally {
        setSaving(null)
      }
    },
    [qc],
  )

  return { save, saving }
}

/**
 * Remove this machine's stored key for a provider.
 *
 * The result says whether anything was actually removed, and what the provider
 * resolves to now — which on a development build can still be "configured",
 * because the repo-root `.env` answers after the keychain does not.
 */
export function useDeleteProviderKey() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (provider: string) => api.providerKeyDelete(provider),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['provider_key_status'] })
      void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
    },
  })
}

/* ── the provider selection ───────────────────────────────────────────────── */

/**
 * Which provider this project has chosen for each capability.
 *
 * The counterpart to `useProviderKeys`, and kept apart from it on purpose: this
 * one is a property of the *project folder* and travels to everyone on the
 * share, that one is a property of *this machine* and travels nowhere. The pane
 * renders them as two bands for the same reason.
 *
 * Not part of `invalidateWorld`: a collaborator editing a node does not change
 * what `project.json` selects, and the only thing that changes it here is the
 * mutation below.
 */
export function useProviderSelections(): UseQueryResult<ProviderSelections> {
  return useQuery({
    queryKey: qk.projectProviders,
    queryFn: api.projectProviders,
    retry: false,
  })
}

export const BACKEND_HEALTH_POLL_MS = 5_000

export const BACKEND_HEALTH_MAX_BACKOFF_MS = 60_000

interface BackendPollState {
  project: string
  /** A non-empty external queue is the only reason to keep probing. */
  liveQueue: boolean
  failures: number
  dataUpdates: number
  errorUpdates: number
}

function backendPollInterval(
  query: {
    state: {
      data?: api.StatusBarBackend
      dataUpdateCount: number
      errorUpdateCount: number
    }
  },
  poll: BackendPollState,
): number | false {
  const { state } = query

  if (state.dataUpdateCount !== poll.dataUpdates) {
    poll.dataUpdates = state.dataUpdateCount
    const health = state.data?.health
    if (health?.state === 'connected') {
      poll.liveQueue = health.externalQueue !== null && health.externalQueue > 0
      poll.failures = 0
    } else if (health?.state === 'unavailable' && poll.liveQueue) {
      poll.failures += 1
    } else {
      poll.liveQueue = false
      poll.failures = 0
    }
  }

  // A bridge/network rejection has no fresh health payload. If a queue was
  // live immediately before it, retain that fact and back off until a check
  // succeeds; otherwise an idle error must not start a polling loop.
  if (state.errorUpdateCount !== poll.errorUpdates) {
    poll.failures += poll.liveQueue ? 1 : 0
    poll.errorUpdates = state.errorUpdateCount
  }

  if (!poll.liveQueue) return false
  return Math.min(
    BACKEND_HEALTH_POLL_MS * 2 ** Math.max(0, poll.failures - 1),
    BACKEND_HEALTH_MAX_BACKOFF_MS,
  )
}

interface BackendHealthSubscription {
  references: number
  dispose: () => void
}

const backendHealthSubscriptions = new WeakMap<QueryClient, BackendHealthSubscription>()

/**
 * One event bridge per QueryClient, even though both Workspace and Inspector
 * observe the same health query. Job outcomes are meaningful health signals;
 * they replace the old unconditional idle poll without doubling requests.
 */
function subscribeBackendHealth(qc: QueryClient): () => void {
  const existing = backendHealthSubscriptions.get(qc)
  if (existing) {
    existing.references += 1
    return () => {
      existing.references -= 1
      if (existing.references === 0) existing.dispose()
    }
  }

  let disposed = false
  let pendingWhileHidden = false
  const unlisteners: Array<() => void> = []
  const refresh = () => {
    if (document.visibilityState === 'hidden') {
      pendingWhileHidden = true
      return
    }
    pendingWhileHidden = false
    void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
  }
  const onVisible = () => {
    if (document.visibilityState === 'visible' && pendingWhileHidden) refresh()
  }
  window.addEventListener('online', refresh)
  document.addEventListener('visibilitychange', onVisible)

  const attach = <T>(name: string, handler: (payload: T) => void) => {
    if (!api.isTauri()) return
    void listen<T>(name, (event) => handler(event.payload))
      .then((unlisten) => {
        if (disposed) unlisten()
        else unlisteners.push(unlisten)
      })
      .catch(() => {
        /* explicit refresh and queue-aware polling remain available */
      })
  }

  // A running transition is the actual provider attempt, rather than merely a
  // button press or a queued local job. Track attempts so unrelated queue
  // transitions do not repeatedly probe health.
  const attempts = new Map<string, number>()
  attach<QueueSnapshot>(api.JOB_EVENTS.state, (snapshot) => {
    let attempted = false
    const present = new Set<string>()
    for (const job of snapshot.jobs) {
      if (job.kind !== 'generate') continue
      present.add(job.id)
      if (job.state === 'running' && attempts.get(job.id) !== job.attempt) {
        attempts.set(job.id, job.attempt)
        attempted = true
      }
    }
    for (const id of attempts.keys()) if (!present.has(id)) attempts.delete(id)
    if (attempted) refresh()
  })
  attach<api.JobFailed>(api.JOB_EVENTS.error, (failure) => {
    if (failure.kind === 'generate') refresh()
  })
  attach<api.JobDone>(api.JOB_EVENTS.done, (done) => {
    if (done.kind === 'generate') refresh()
  })

  const subscription: BackendHealthSubscription = {
    references: 1,
    dispose: () => {
      if (subscription.references !== 0) return
      disposed = true
      window.removeEventListener('online', refresh)
      document.removeEventListener('visibilitychange', onVisible)
      for (const unlisten of unlisteners) unlisten()
      backendHealthSubscriptions.delete(qc)
    },
  }
  backendHealthSubscriptions.set(qc, subscription)

  return () => {
    subscription.references -= 1
    if (subscription.references === 0) subscription.dispose()
  }
}

/**
 * Models resolved by the backend plus a real, non-generating health check.
 * The project id belongs in the cache key even though the command needs no
 * argument: the command reads the open project, while this cache can outlive a
 * Workspace during a project switch.
 */
export function useStatusBarBackend(project: string): UseQueryResult<api.StatusBarBackend> {
  const qc = useQueryClient()
  const poll = useMemo<BackendPollState>(
    () => ({ project, liveQueue: false, failures: 0, dataUpdates: 0, errorUpdates: 0 }),
    [project],
  )
  const query = useQuery({
    queryKey: qk.statusBarBackend(project),
    queryFn: api.statusBarBackend,
    // Gemini reports no external queue, and an idle ComfyUI reports zero: both
    // stop here after the initial/event-driven check. Only a queue whose depth
    // is visibly changing justifies polling the remote backend.
    refetchInterval: (current) => backendPollInterval(current, poll),
    refetchIntervalInBackground: false,
    retry: false,
  })

  useEffect(() => subscribeBackendHealth(qc), [qc])
  return query
}

/**
 * Choose a provider for one capability.
 *
 * The result is the whole selection map, written into the cache rather than
 * refetched: the backend has just re-read `project.json` to answer, so a second
 * round trip would only ask the same question again.
 *
 * Rejects with `write.read_only` on a read-only folder, which the pane already
 * knows and disables for — the rejection is the backstop for a folder that
 * turned read-only mid-session.
 */
export function useSelectProvider() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (v: {
      capability: Capability
      provider: string
      model?: string
      region?: string
    }) => api.projectProviderSelect(v.capability, v.provider, v.model, v.region),
    onSuccess: (selections) => {
      qc.setQueryData(qk.projectProviders, selections)
      void qc.invalidateQueries({ queryKey: ['status_bar_backend'] })
      void qc.invalidateQueries({ queryKey: ['image_reference_report'] })
      void qc.invalidateQueries({ queryKey: ['image_generation_capabilities'] })
    },
    onError: (e: unknown) => report(e, 'Could not change the provider'),
  })
}

/* ── undo ─────────────────────────────────────────────────────────────────── */
