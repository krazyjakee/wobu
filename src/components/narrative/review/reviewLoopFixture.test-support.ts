/** Scripted IPC for component tests and clearly labelled browser proof. */
import { mockIPC } from '@tauri-apps/api/mocks'
import type { Scene } from '../../../lib/api'
import type { ReviewRequest } from '../../../lib/api/narrativeReview'
import type { GenerationHistory } from '../../../lib/api/narrativeGeneration'
import { reviewFixture } from './reviewFixture.test-support'
export function reviewLoopFixture() {
  const view = reviewFixture()
  view.lines[0]!.proposals = []
  const history: GenerationHistory[] = []
  const decisions: ReviewRequest[] = []
  let generation = 0
  let revision = 0
  const line = () => view.lines[0]!
  const scene = (): Scene => ({
    id: view.scene_id,
    name: 'Beacon aftermath',
    beats: [
      {
        id: 'arrival',
        title: 'Beacon failure',
        dialogue: [
          {
            id: 'warning',
            speaker: line().speaker,
            policy: line().slot_policy,
            variants: [{ id: 'warning-main', text: line().text! }],
          },
        ],
      },
    ],
  })
  const advance = () => {
    revision++
    view.guard = { stamp: null, head: `head-${revision}` }
  }
  mockIPC((command, args) => {
    const payload = args as Record<string, unknown>
    switch (command) {
      case 'narrative_state_get':
        return { document: { schema_version: 1, variables: [] }, stamp: null }
      case 'job_list':
        return { jobs: [], queued: 0, running: 0, retrying: 0 }
      case 'narrative_generation_history':
        return structuredClone(history)
      case 'narrative_generation_plan':
        return {
          id: 'batch',
          provider: 'fixture-provider',
          model: 'scripted-not-live',
          requests: [
            {
              request_id: 'request',
              target: line().target,
              candidate_variant_id: 'warning-main',
              expected_policy: line().text!.lifecycle!.policy,
              context: { estimated_tokens: 100 },
              prompt: 'Frozen fixture context',
            },
          ],
          skipped: [],
        }
      case 'narrative_generation_start': {
        generation++
        const candidate = {
          slot_id: 'warning',
          variant_id: 'warning-main',
          speaker: line().speaker,
          text:
            generation === 1
              ? 'I watched the beacon go dark.'
              : 'I saw the last light die. We were too late.',
        }
        history.push({
          request_id: `request-${generation}`,
          batch_id: 'batch',
          target: line().target,
          provider: 'fixture-provider',
          model: 'scripted-not-live',
          attempts: 1,
          status: 'succeeded',
          receipt_id: `receipt-${generation}`,
          candidate,
          usage: { input: 100, cached_input: 0, output: 20 },
          billing_unknown: false,
          error_code: null,
          proposal_published: true,
          proposal_current_at_publication: true,
        })
        if (generation === 1) {
          line().text!.body = candidate.text
          line().text!.revision = 'generated-revision'
          advance()
        } else {
          line().proposals.push({
            id: 'proposal',
            hash: 'proposal-hash',
            request_id: `request-${generation}`,
            receipt_id: `receipt-${generation}`,
            candidate,
            base_revision: line().text!.revision,
            base_wording: line().text!.body,
            status: 'pending',
            reason: 'Explicit regeneration of Edited wording; accepted words were retained.',
          })
        }
        return []
      }
      case 'narrative_review_list':
        return structuredClone({
          scenes: [view],
          errors: [],
          next_offset: null,
          total_scenes: 1,
          catalog_revision: 'catalog',
        })
      case 'narrative_review_context':
        return {
          version: 1,
          revision: line().context_revision,
          state: {},
          inputs: { facts: ['beacon_failed'], knowledge: { mara: 'witnessed' } },
        }
      case 'narrative_review_apply': {
        const request = payload.request as ReviewRequest
        if (request.guard.head !== view.guard.head)
          throw new Error('Source changed. Reload and compare.')
        decisions.push(structuredClone(request))
        switch (request.action.kind) {
          case 'edit':
            line().text!.body = request.action.body
            line().text!.revision = `edited-${revision}`
            line().text!.lifecycle!.policy = 'edited'
            break
          case 'accept':
            line().text!.body = request.action.reviewed_text ?? line().proposals[0]!.candidate.text
            line().text!.revision = `accepted-${revision}`
            line().text!.lifecycle!.policy = 'edited'
            line().proposals[0]!.status = 'accepted'
            break
          case 'approve':
          case 'attest':
            line().review = 'approved'
            line().text!.lifecycle!.review = 'approved'
            line().approval_valid = true
            line().freshness = 'current'
            break
          case 'policy':
            if (request.action.scope === 'slot') line().slot_policy = request.action.policy
            else line().text!.lifecycle!.policy = request.action.policy
            break
          case 'reject':
            line().proposals[0]!.status = 'rejected'
            break
        }
        advance()
        view.history.unshift({
          id: `event-${revision}`,
          action: request.action.kind,
          target: request.target,
          actor: 'Fixture editor',
          context_revision: request.context_revision,
        })
        return structuredClone({
          file: {
            scene: scene(),
            slug: 'beacon-aftermath',
            rel: 'narrative/scenes/beacon-aftermath.scene.yaml',
            stamp: null,
          },
          review: view,
        })
      }
      default:
        throw new Error(`Unexpected fixture IPC: ${command}`)
    }
  })
  return {
    scene,
    decisions,
    view,
    changeContext: () => {
      advance()
      line().context_revision = 'changed-context'
      line().freshness = 'out_of_date'
      line().approval_valid = false
      line().reason = 'Character knowledge changed after approval. Review the new context.'
    },
  }
}
