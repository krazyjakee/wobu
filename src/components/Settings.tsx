import type { ProjectSummary } from '../lib/api'
import { useOnboarding } from '../store/onboarding'
import { Icon } from './Icon'
import { KeybindingsSection } from './KeybindingsSection'
import { LegalSection } from './LegalSection'
import { LicencesSection } from './LicencesSection'
import McpSection from './McpSection'
import { UpdateSection } from './UpdateSection'
import { WikiExportSection } from './WikiExportSection'
import { Diagnostics } from './settings/diagnostics'
import { About, Appearance, EditorPrefs, Storage } from './settings/panels'
import { Providers } from './settings/providers'

/**
 * The Settings surface.
 *
 * Sections are independent and each owns its own loading, so a new pane drops
 * in as one more `<section className="set-sec">` rather than a rewrite. Nothing
 * here is stubbed: a control that looks configurable but is not is worse than
 * an honest absence, which is why there is no theme switch — see Appearance.
 */
export function Settings({ project }: { project?: ProjectSummary }) {
  return (
    <div className="settings-mode">
      <div className="settings">
        <h2>Settings</h2>
        <Providers />
        <LegalSection />
        <section className="set-sec">
          <h3>Introduction</h3>
          <p className="set-note">
            The first-run walkthrough — what Wobu is, where a project lives, and how a first concept
            gets made. Running it again changes nothing: your agreement to the terms stays recorded.
          </p>
          <div className="set-acts">
            <button className="btn-mini" onClick={() => useOnboarding.getState().restart()}>
              <Icon name="spark" size="sm" />
              Show the introduction
            </button>
          </div>
        </section>
        <McpSection />
        <Storage />
        <EditorPrefs />
        <Appearance />
        <KeybindingsSection />
        {project && <WikiExportSection project={project} />}
        <Diagnostics />
        <About />
        {/* Directly under About: the version shown there is the thing this pane
            changes, and "which version am I on" is the question that sends
            someone looking for an update in the first place. */}
        <UpdateSection />
        <LicencesSection />
      </div>
    </div>
  )
}

/* ── providers ────────────────────────────────────────────────────────────── */
