import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import * as api from '../../lib/api'
import {
  useAcceptEnhanced,
  useDiscardEnhanced,
  useEnhance,
  useEnhancePending,
  useEnhanceStream,
} from '../../lib/queries'
import { report } from '../../store/ui'

export type EnhanceCandidate = api.EnhanceReady | api.EnhanceDelta

export interface EnhanceSession {
  active: boolean
  candidate: EnhanceCandidate | null
  complete: boolean
  running: boolean
  stopped: boolean
  failure: api.JobFailure | null
  starting: boolean
  accepting: boolean
  discarding: boolean
  refusedNode: api.WobuNode | null
  start: () => void
  stop: () => void
  accept: (description: api.WobuDescription) => void
  forceAccept: () => void
  reject: () => void
}

export function useEnhanceSession(nodeId: string | null, queue: api.QueueSnapshot): EnhanceSession {
  const startMutation = useEnhance()
  const acceptMutation = useAcceptEnhanced()
  const discardMutation = useDiscardEnhanced()
  const pending = useEnhancePending(nodeId !== null)
  const [jobsByNode, setJobsByNode] = useState<Record<string, string>>({})
  const [readyByNode, setReadyByNode] = useState<Record<string, api.EnhanceReady>>({})
  const [hiddenJobs, setHiddenJobs] = useState<Set<string>>(() => new Set())
  const [stoppedJobs, setStoppedJobs] = useState<Set<string>>(() => new Set())
  const [startingNode, setStartingNode] = useState<string | null>(null)
  const [refusedNode, setRefusedNode] = useState<api.WobuNode | null>(null)
  const [submitted, setSubmitted] = useState<{
    jobId: string
    description: api.WobuDescription
  } | null>(null)

  // Listen before a job starts, rather than after its id comes back. A fast
  // provider is allowed to finish between those two renders, and its paid-for
  // result must still reach the review surface.
  useEffect(() => {
    if (!api.isTauri()) return
    let disposed = false
    let unlisten: (() => void) | undefined
    void listen<api.JobDone>(api.JOB_EVENTS.done, (event) => {
      const done = event.payload
      const result = done.result
      if (done.kind !== 'enhance' || !isEnhanceReady(result)) return
      setReadyByNode((current) => ({ ...current, [result.nodeId]: result }))
      setJobsByNode((current) => ({ ...current, [result.nodeId]: result.jobId }))
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => {
        /* the pending query is the reload/catch-up path */
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  useEffect(() => {
    setRefusedNode(null)
    setSubmitted(null)
  }, [nodeId])

  const liveReady = nodeId ? readyByNode[nodeId] : undefined
  const caughtUpReady = nodeId
    ? [...(pending.data ?? [])]
        .reverse()
        .find((ready) => ready.nodeId === nodeId && !hiddenJobs.has(ready.jobId))
    : undefined
  const ready = liveReady && !hiddenJobs.has(liveReady.jobId) ? liveReady : (caughtUpReady ?? null)
  const jobId = nodeId ? (jobsByNode[nodeId] ?? ready?.jobId ?? null) : null
  const stream = useEnhanceStream(jobId)
  const partial =
    stream && stream.nodeId === nodeId && !hiddenJobs.has(stream.jobId) ? stream : null
  const candidate: EnhanceCandidate | null = ready ?? partial
  const job = jobId ? queue.jobs.find((item) => item.id === jobId) : undefined
  const stopped = Boolean(jobId && (stoppedJobs.has(jobId) || job?.state === 'cancelled'))
  const running =
    Boolean(jobId) &&
    !ready &&
    !stopped &&
    (!job || job.state === 'queued' || job.state === 'running' || job.state === 'retrying')
  const failure = job?.state === 'failed' ? job.failure : null

  const hide = (id: string) => {
    setHiddenJobs((current) => new Set(current).add(id))
    setRefusedNode(null)
    setSubmitted(null)
  }

  const start = () => {
    if (!nodeId || startMutation.isPending) return
    setStartingNode(nodeId)
    setRefusedNode(null)
    setSubmitted(null)
    startMutation.mutate(nodeId, {
      onSuccess: (id) => {
        setJobsByNode((current) => ({ ...current, [nodeId]: id }))
        setStartingNode(null)
      },
      onError: (error) => {
        setStartingNode(null)
        report(error, 'Could not start Enhance')
      },
    })
  }

  const stop = () => {
    if (!jobId) return
    setStoppedJobs((current) => new Set(current).add(jobId))
    void api.jobCancel(jobId).catch((error: unknown) => report(error, 'Could not stop Enhance'))
  }

  const submit = (description: api.WobuDescription, force: boolean) => {
    if (!ready) return
    setSubmitted({ jobId: ready.jobId, description })
    acceptMutation.mutate(
      { jobId: ready.jobId, description, force },
      {
        onSuccess: (accepted) => {
          if (accepted.outcome === 'refusedEdit') {
            setRefusedNode(accepted.node)
          } else {
            hide(ready.jobId)
          }
        },
        onError: (error) => report(error, 'Could not accept enhanced description'),
      },
    )
  }

  const reject = () => {
    if (!candidate) {
      if (jobId) hide(jobId)
      return
    }
    if (!ready) {
      hide(candidate.jobId)
      return
    }
    discardMutation.mutate(ready.jobId, {
      onSuccess: () => hide(ready.jobId),
      onError: (error) => report(error, 'Could not reject enhanced description'),
    })
  }

  return {
    active: startingNode === nodeId || Boolean(jobId && !hiddenJobs.has(jobId)),
    candidate,
    complete: ready !== null,
    running,
    stopped,
    failure,
    starting: startingNode === nodeId,
    accepting: acceptMutation.isPending,
    discarding: discardMutation.isPending,
    refusedNode,
    start,
    stop,
    accept: (description) => submit(description, false),
    forceAccept: () => {
      if (submitted) submit(submitted.description, true)
    },
    reject,
  }
}

function isEnhanceReady(value: unknown): value is api.EnhanceReady {
  if (!value || typeof value !== 'object') return false
  const ready = value as Partial<api.EnhanceReady>
  return (
    typeof ready.jobId === 'string' &&
    typeof ready.nodeId === 'string' &&
    Boolean(ready.description && typeof ready.description === 'object') &&
    Array.isArray(ready.questions)
  )
}
