import { useEffect, useState, type ReactNode } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import privacyPolicy from '../../docs/legal/privacy-policy.md?raw'
import termsOfUse from '../../docs/legal/terms.md?raw'
import type { BackendHealth, ProjectSummary } from '../lib/api'
import { useCurrentProject, useOpenProject, useStatusBarBackend } from '../lib/queries'
import { chordParts } from '../lib/keys'
import { closeWindow } from '../lib/window'
import { bindingOf, useKeybindings, type CommandId } from '../store/keybindings'
import { LEGAL_VERSION, ONBOARDING_STEPS, useOnboarding } from '../store/onboarding'
import { report, useUI } from '../store/ui'
import { Icon } from './Icon'
import { Modal } from './Modal'
import { NewProjectSheet } from './Launcher'

/**
 * First run: what Wobu is, where the work lives, and the honest path to a first
 * generated image.
 *
 * Mounted from `App` rather than from `Launcher` on purpose. The flow does not
 * end when a project opens — creating one is the *middle* of it, and the two
 * steps that matter most (a provider key, and the sequence that produces a
 * concept) can only be described once there is a workspace to describe. An
 * overlay owned by the launcher would unmount at exactly the point the user
 * stopped knowing what to do next.
 *
 * Two rules run through the copy. It never names a surface this build does not
 * have — Board and the History tab were removed in #144 and appear nowhere
 * here. And it never hard-codes a keystroke: every chord below is read from the
 * registry in `store/keybindings.ts`, so a rebound key is right in the tour the
 * moment it changes, and the tour ends by pointing at the full reference rather
 * than trying to be it.
 *
 * Above all it does not promise a generation that cannot happen. Wobu is BYOK
 * (`docs/08-providers.md`): a machine with no key and no ComfyUI cannot reach a
 * first concept, and a tour that ended on "now press Generate" would be lying
 * to precisely the person it exists to help. The last step reads the same
 * backend check the status bar does and says which of the two situations the
 * reader is in.
 */
export function OnboardingOverlay() {
  const load = useOnboarding((s) => s.load)
  const open = useOnboarding((s) => s.open)
  const record = useOnboarding((s) => s.record)

  useEffect(() => void load(), [load])

  // `record` is null until the core has answered. Drawing a welcome screen and
  // then snatching it back from someone who settled all this months ago is
  // worse than the frame of nothing this costs.
  if (!open || record === null) return null
  return <OpenOnboarding />
}

function OpenOnboarding() {
  const step = useOnboarding((s) => s.step)
  const go = useOnboarding((s) => s.go)
  const dismiss = useOnboarding((s) => s.dismiss)
  const project = useCurrentProject().data ?? null

  const index = ONBOARDING_STEPS.indexOf(step)
  const previous = index > 1 ? ONBOARDING_STEPS[index - 1] : undefined
  const next = ONBOARDING_STEPS[index + 1]

  return (
    <Modal
      titleId="onboarding-title"
      descriptionId="onboarding-description"
      className="sheet onb"
      closeOnBackdrop={false}
      // Escape is the same answer as Skip everywhere except the gate, where
      // there is nothing to skip to: `LegalStep` renders its own footer and
      // this handler is a no-op behind it.
      onClose={() => {
        if (step !== 'legal') void dismiss()
      }}
    >
      <div className="onb-head">
        <span className="brand-mark brand-lg" />
        <div>
          <h2 id="onboarding-title">{step === 'legal' ? 'Before you start' : 'Welcome to wobu'}</h2>
          <p id="onboarding-description">
            {step === 'legal'
              ? 'Two documents, and then nothing else stands between you and the app.'
              : 'Author the hierarchy once. Every generation inherits it.'}
          </p>
        </div>
      </div>

      {step === 'legal' ? (
        <LegalStep />
      ) : (
        <>
          <ol className="onb-steps" aria-label="Introduction progress">
            {ONBOARDING_STEPS.slice(1).map((id) => (
              <li key={id} className={id === step ? 'is-on' : undefined} aria-current={id === step}>
                <button onClick={() => go(id)}>{STEP_LABELS[id]}</button>
              </li>
            ))}
          </ol>

          <div className="onb-body">
            {step === 'welcome' && <WelcomeStep />}
            {step === 'project' && <ProjectStep project={project} />}
            {step === 'providers' && <ProvidersStep project={project} />}
            {step === 'concept' && <ConceptStep project={project} />}
          </div>

          <div className="onb-foot">
            <button className="btn btn-ghost" onClick={() => void dismiss()}>
              {next ? 'Skip for now' : 'Close'}
            </button>
            <div className="tspace" />
            {previous && (
              <button className="btn" onClick={() => go(previous)}>
                Back
              </button>
            )}
            {next ? (
              <button className="btn btn-primary" onClick={() => go(next)}>
                Next
              </button>
            ) : (
              <FinishButton project={project} />
            )}
          </div>
        </>
      )}
    </Modal>
  )
}

const STEP_LABELS: Record<string, string> = {
  welcome: 'What this is',
  project: 'Your project',
  providers: 'Your keys',
  concept: 'First concept',
}

/* ── the gate ─────────────────────────────────────────────────────────────── */

const DOCUMENTS = [
  { id: 'terms', label: 'Terms of use', text: termsOfUse },
  { id: 'privacy', label: 'Privacy policy', text: privacyPolicy },
] as const

/**
 * The one step with no Skip.
 *
 * #137 shipped both documents and put them in Settings, but left the acceptance
 * itself unbuilt because there was no first run to hang it on. This is that
 * hook. It is a gate rather than a notice: the button says what it does, the
 * full text of both documents is one click away *in the app* rather than behind
 * a link to a website, and the alternative to agreeing is offered plainly
 * instead of being left as "close the window somehow".
 */
function LegalStep() {
  const acceptLegal = useOnboarding((s) => s.acceptLegal)
  const saving = useOnboarding((s) => s.saving)
  const [shown, setShown] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)

  const document = DOCUMENTS.find((d) => d.id === shown) ?? null

  async function accept() {
    setFailed(false)
    if (!(await acceptLegal())) setFailed(true)
  }

  return (
    <>
      <div className="onb-body">
        <p>
          Wobu is free software under the MIT licence. The two documents below are the ones the
          installer drops beside the application; the copies here are read from the same files, so
          what you agree to is what is on your disk.
        </p>
        <ul className="onb-facts">
          <li>No telemetry of any kind. Nothing is ever reported back to us.</li>
          <li>Your world stays in the folder you point Wobu at, as readable Markdown.</li>
          <li>
            Your provider keys stay in this computer&rsquo;s keychain and are never written into a
            project.
          </li>
        </ul>

        <div className="onb-docs">
          {DOCUMENTS.map((d) => (
            <button
              key={d.id}
              className={shown === d.id ? 'btn-mini is-on' : 'btn-mini'}
              aria-pressed={shown === d.id}
              onClick={() => setShown((current) => (current === d.id ? null : d.id))}
            >
              <Icon name={d.id === 'terms' ? 'library' : 'lock'} size="sm" />
              {shown === d.id ? `Hide ${d.label.toLowerCase()}` : d.label}
            </button>
          ))}
        </div>

        {document && (
          <pre className="onb-doc" tabIndex={0} aria-label={document.label}>
            {document.text.trim()}
          </pre>
        )}

        <p className="onb-rev">Revision on this build — {LEGAL_VERSION}.</p>
        {failed && (
          <p className="onb-err" role="alert">
            Your agreement could not be written to this computer&rsquo;s Wobu settings, so it has
            not been recorded. Check that the application-data folder is writable and try again.
          </p>
        )}
      </div>

      <div className="onb-foot">
        <button className="btn btn-ghost" onClick={() => void closeWindow()} disabled={saving}>
          Quit without agreeing
        </button>
        <div className="tspace" />
        <button className="btn btn-primary" onClick={() => void accept()} disabled={saving}>
          {saving ? 'Recording…' : 'I agree — continue'}
        </button>
      </div>
    </>
  )
}

/* ── the tour ─────────────────────────────────────────────────────────────── */

function WelcomeStep() {
  return (
    <>
      <p>
        A world is a hierarchy before it is a picture. Wobu is where you write that hierarchy down
        once — the art style, the lore, a species, its cultures, the places and the characters — and
        every image you generate afterwards inherits it, because the prompt is compiled from the
        node&rsquo;s ancestors rather than retyped.
      </p>
      <ul className="onb-rows">
        <ModeRow icon="library" name="Library">
          Where the writing happens. A tree of nodes on the left, the node&rsquo;s notes and
          concepts in the middle, and what its prompt actually inherits on the right.
        </ModeRow>
        <ModeRow icon="forge" name="Forge">
          Several nodes in one shot — put a character in a place and compose the scene.
        </ModeRow>
        <ModeRow icon="assets" name="Assets">
          Every image, reference and mesh in the project folder, in one grid.
        </ModeRow>
        <ModeRow icon="settings" name="Settings">
          Keys, providers, keyboard, and this introduction again whenever you want it.
        </ModeRow>
      </ul>
    </>
  )
}

function ModeRow({ icon, name, children }: { icon: string; name: string; children: ReactNode }) {
  return (
    <li>
      <Icon name={icon} size="sm" />
      <span>
        <b>{name}</b> — {children}
      </span>
    </li>
  )
}

/** The same words the launcher's own picker uses, so the dialog is familiar. */
const DIALOG_TITLE = 'Open a Wobu project folder'

function ProjectStep({ project }: { project: ProjectSummary | null }) {
  const openProject = useOpenProject()
  const [newOpen, setNewOpen] = useState(false)

  async function openFolder() {
    try {
      const picked = await openDialog({ directory: true, multiple: false, title: DIALOG_TITLE })
      if (typeof picked === 'string') openProject.mutate(picked, { onError: (e) => report(e) })
    } catch (e) {
      report(e)
    }
  }

  return (
    <>
      <p>
        A project is an ordinary folder, not a database file. Nodes are Markdown you can read in any
        editor, references and generated images sit beside them, and the whole thing can live on a
        network share — anyone who can see the path can open it, on their own machine, with their
        own keys.
      </p>

      {project ? (
        <Done label={project.name}>
          Open now, at <code>{project.path}</code>. Everything Wobu writes goes inside it.
        </Done>
      ) : (
        <>
          <p className="onb-note">
            Nothing is open yet. Create one anywhere you keep work — Wobu never hides files
            elsewhere.
          </p>
          <div className="onb-acts">
            <button
              className="btn"
              onClick={() => void openFolder()}
              disabled={openProject.isPending}
            >
              <Icon name="folder" size="sm" />
              Open folder…
            </button>
            <button
              className="btn btn-primary"
              onClick={() => setNewOpen(true)}
              disabled={openProject.isPending}
            >
              <Icon name="plus" size="sm" />
              New project
            </button>
          </div>
        </>
      )}

      {newOpen && <NewProjectSheet onClose={() => setNewOpen(false)} />}
    </>
  )
}

/**
 * BYOK, said before it is discovered.
 *
 * The dead end this step exists to remove is specific: a first-time user
 * presses Generate, waits, and gets `provider.no_key` from a provider they were
 * never asked about. Saying it here costs one screen; saying it at generate
 * time costs the first impression.
 */
function ProvidersStep({ project }: { project: ProjectSummary | null }) {
  return (
    <>
      <p>
        Wobu runs no inference of its own and proxies nothing through us. You bring your own keys:
        an Anthropic or Gemini key for writing, and for images either a Gemini key or a ComfyUI you
        run yourself. They go into this computer&rsquo;s keychain, never into the project folder, so
        a world you share carries the <i>choice</i> of provider and none of your credentials.
      </p>
      {project ? (
        <BackendState project={project} />
      ) : (
        <p className="onb-note">
          Settings lives inside an open project, so this is the step after the last one. Once a
          project is open, Settings is on the mode rail — or press <Chord id="mode.settings" />.
        </p>
      )}
      <p className="onb-note">
        Without a key Wobu is still a perfectly good place to write a world; it just cannot draw it
        yet. Nothing else is disabled.
      </p>
    </>
  )
}

function ConceptStep({ project }: { project: ProjectSummary | null }) {
  return (
    <>
      <p>Three moves, in the Library:</p>
      <ol className="onb-numbered">
        <li>
          <b>Make a node.</b> <Chord id="node.new" /> or <b>New</b> in the navigator. A character, a
          place, a species — whatever you have in your head first.
        </li>
        <li>
          <b>Give it its world.</b> Nest it under a parent and pin reference images in the
          inspector. The inspector shows the compiled prompt, so you can see exactly what the
          ancestors contributed before spending anything.
        </li>
        <li>
          <b>Generate.</b> The <i>Concepts</i> tab on the node, or <Chord id="generate" />. The
          result is written into the project folder beside the notes.
        </li>
      </ol>
      {project ? (
        <BackendState project={project} />
      ) : (
        <p className="onb-note">Open a project first — step two above.</p>
      )}
      <p className="onb-note">
        Every key in the app is listed under <Chord id="shortcuts.show" />, and can be rebound in
        Settings.
      </p>
    </>
  )
}

/**
 * The finishing control, which is only "Finish" when finishing is the truthful
 * thing to offer. When the project has no usable image backend the primary
 * action is the one the reader actually needs next, and it lands them on the
 * pane rather than describing where it is.
 */
function FinishButton({ project }: { project: ProjectSummary | null }) {
  // Without a project there is no backend to ask about, and the hook below
  // would poll a command that has nothing to answer for.
  if (project === null) return <Finish />
  return <ProjectFinishButton project={project} />
}

function ProjectFinishButton({ project }: { project: ProjectSummary }) {
  const dismiss = useOnboarding((s) => s.dismiss)
  const backend = useStatusBarBackend(project.id)
  if (backend.data?.health.state === 'connected') return <Finish />
  return (
    <button
      className="btn btn-primary"
      onClick={() => {
        useUI.getState().setMode('settings')
        void dismiss()
      }}
    >
      <Icon name="settings" size="sm" />
      Set up a provider
    </button>
  )
}

function Finish() {
  const dismiss = useOnboarding((s) => s.dismiss)
  return (
    <button className="btn btn-primary" onClick={() => void dismiss()}>
      Finish
    </button>
  )
}

/** The status bar's backend check, in a sentence rather than a chip. */
function BackendState({ project }: { project: ProjectSummary }) {
  const backend = useStatusBarBackend(project.id)
  if (backend.isPending) return <p className="onb-note">Checking what this project can generate…</p>
  if (backend.isError) {
    return <p className="onb-note">The image backend could not be checked from here.</p>
  }
  const health = backend.data?.health
  if (!health) return null
  const label = backend.data?.image?.label ?? 'the selected provider'
  if (health.state === 'connected') {
    return <Done label={`${label} is ready`}>Generate will work on this machine.</Done>
  }
  return (
    <p className="onb-warn">
      <Icon name="lock" size="sm" />
      <span>{healthText(health, label)}</span>
    </p>
  )
}

function healthText(health: BackendHealth, label: string): string {
  if (health.state === 'connected') return `${label} is ready.`
  if (health.state === 'unconfigured') {
    return `${health.detail} Choose one under Settings › Providers and add its key.`
  }
  return `${label} cannot be reached from here — ${health.detail}`
}

function Done({ label, children }: { label: string; children: ReactNode }) {
  return (
    <p className="onb-done">
      <Icon name="check" size="sm" />
      <span>
        <b>{label}</b> {children}
      </span>
    </p>
  )
}

/** One chord, read from the registry so a rebound key is never wrong here. */
function Chord({ id }: { id: CommandId }) {
  const chord = useKeybindings((s) => bindingOf(s.overrides, id))
  if (!chord) return <span className="keys-unbound">an unbound key</span>
  return (
    <span className="keys-chord">
      {chordParts(chord).map((part, i) => (
        <kbd key={i}>{part}</kbd>
      ))}
    </span>
  )
}
