//! Settings that belong to this Wobu installation, never to an open project.
//!
//! A ComfyUI address is not a project choice. `project.json` may say that a
//! world uses ComfyUI, but the server under one collaborator's desk is not the
//! server another collaborator can reach. The endpoint therefore lives in
//! `app_data_dir()/settings.json`, beside the recents list and local indexes,
//! and is managed separately from [`crate::state::AppState`].
//!
//! ## Trust boundary
//!
//! Loopback is the safe default. Choosing a LAN or HTTPS address authorises the
//! Rust process to send that server prompts and reference-image bytes for image,
//! scene-composition, replay and local-mesh jobs. Only HTTP(S) is accepted;
//! URL credentials, query strings and fragments are rejected so a password
//! cannot be persisted here or accidentally echoed back to the webview/log.
//! Authentication remains the reverse proxy's concern, and the endpoint probe
//! reports 401/403 distinctly rather than asking for a credential Wobu has no
//! safe storage or request-header contract for.

use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::State;
use url::Url;
use wobu_imagine::{ComfyBackend, ComfyMeshBackend, Error as ImageError, comfy};
use wobu_store::paths;

use crate::error::{Code, CommandResult, WobuError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    comfyui_endpoint: String,
    /// `default` rather than required: every `settings.json` written before
    /// first-run onboarding existed has no such key, and an installation that
    /// predates the gate has genuinely accepted nothing.
    #[serde(default)]
    onboarding: OnboardingState,
}

impl Default for Stored {
    fn default() -> Self {
        Self {
            comfyui_endpoint: comfy::DEFAULT_URL.to_owned(),
            onboarding: OnboardingState::default(),
        }
    }
}

/// What the first run settled, as durable as the ComfyUI route beside it.
///
/// Here rather than in the project folder for exactly the reason the endpoint
/// is: accepting the terms is a fact about this installation and the person at
/// this keyboard, not about a world that gets copied onto a share. Writing it
/// into `project.json` would mean one author's acceptance travelled to every
/// collaborator who opened the folder, which is not what any of them agreed to.
///
/// Three separate `Option`s rather than one boolean because the questions are
/// separate. "Have the documents been accepted" gates the app; "which revision"
/// is what lets a future edit to `docs/legal/*.md` ask again rather than
/// silently inherit consent for text nobody has read; "was the tour finished"
/// only decides whether an overlay appears. `None` everywhere is a fresh
/// install, and that is also what a deleted `settings.json` reads as.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OnboardingState {
    /// RFC 3339. `None` means the documents have never been accepted here.
    pub legal_accepted_at: Option<String>,
    /// The revision string the webview derived from the two shipped documents.
    pub legal_version: Option<String>,
    /// RFC 3339, set when the tour is finished *or* skipped — both mean "do not
    /// show this again", and recording them as one field is what stops a skip
    /// from being a decision the user has to make on every launch.
    pub completed_at: Option<String>,
}

/// The machine-local settings returned to Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineSettingsView {
    pub comfyui_endpoint: String,
}

/// One probe result. Invalid URLs reject the command; every network answer is
/// data so Settings can keep the explanation beside the field that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComfyEndpointState {
    Connected,
    Unreachable,
    AuthenticationRequired,
    Incompatible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComfyEndpointProbe {
    pub endpoint: String,
    pub state: ComfyEndpointState,
    pub ok: bool,
    pub message: String,
}

/// Loaded once at process start and shared by every provider command.
#[derive(Debug)]
pub struct MachineSettings {
    path: PathBuf,
    stored: RwLock<Stored>,
}

impl Default for MachineSettings {
    fn default() -> Self {
        Self::load()
    }
}

impl MachineSettings {
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    fn load_from(path: PathBuf) -> Self {
        let stored = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Stored>(&raw).ok())
            .map(|stored| {
                // An endpoint this build cannot parse falls back to loopback
                // rather than discarding the whole file. It used to discard it,
                // which was harmless when the file held one URL and is not now:
                // a recorded acceptance must not be undone by a bad address.
                let comfyui_endpoint = normalise_endpoint(&stored.comfyui_endpoint)
                    .unwrap_or_else(|_| comfy::DEFAULT_URL.to_owned());
                Stored { comfyui_endpoint, ..stored }
            })
            .unwrap_or_default();
        Self { path, stored: RwLock::new(stored) }
    }

    /// Read, change, write, publish — under one lock, for the whole file.
    ///
    /// Every mutation goes through here so that none of them can serialise only
    /// the field it happens to know about. The bug this makes impossible is the
    /// obvious one: saving a ComfyUI address rewriting `settings.json` without
    /// the onboarding block, and quietly asking the user to accept the terms
    /// again the next time they launch.
    fn update<T>(&self, change: impl FnOnce(&mut Stored) -> T) -> CommandResult<T> {
        let mut current = self.stored.write();
        let mut next = current.clone();
        let out = change(&mut next);
        save(&self.path, &next)?;
        *current = next;
        Ok(out)
    }

    pub fn view(&self) -> MachineSettingsView {
        MachineSettingsView { comfyui_endpoint: self.comfyui_endpoint() }
    }

    pub fn comfyui_endpoint(&self) -> String {
        self.stored.read().comfyui_endpoint.clone()
    }

    pub fn set_comfyui_endpoint(&self, endpoint: &str) -> CommandResult<MachineSettingsView> {
        let endpoint = normalise_endpoint(endpoint)?;
        self.update(|stored| stored.comfyui_endpoint = endpoint)?;
        Ok(self.view())
    }

    pub fn onboarding(&self) -> OnboardingState {
        self.stored.read().onboarding.clone()
    }

    /// Record that the terms of use and the privacy policy were accepted.
    ///
    /// The timestamp is taken here rather than passed in: the fact being
    /// recorded is what this machine's clock said when the button was pressed,
    /// and a value the webview supplied would be a value the webview could get
    /// wrong. `version` is text the webview *derived* from the two documents it
    /// rendered, which is the one thing the Rust side cannot know — it is the
    /// revision that was actually on screen.
    fn accept_legal(&self, version: String) -> CommandResult<OnboardingState> {
        self.update(|stored| {
            stored.onboarding.legal_accepted_at = Some(now());
            stored.onboarding.legal_version = Some(version);
            stored.onboarding.clone()
        })
    }

    /// Record that the tour is over, however it ended.
    fn finish_onboarding(&self) -> CommandResult<OnboardingState> {
        self.update(|stored| {
            stored.onboarding.completed_at = Some(now());
            stored.onboarding.clone()
        })
    }

    /// The synchronous image factory used by status/capability paths.
    pub fn comfy_image(&self) -> Result<ComfyBackend, ImageError> {
        ComfyBackend::new(self.comfyui_endpoint())
    }

    /// The probed image factory used before a generation, composition or
    /// replay is queued.
    pub async fn connect_comfy_image(&self) -> Result<ComfyBackend, ImageError> {
        ComfyBackend::connect(self.comfyui_endpoint()).await
    }

    /// Kept beside the image factory so local mesh can never grow its own URL
    /// default. Both are pinned by the routing test below. The current shell
    /// can display stored mesh concepts but has not wired its mesh queue command
    /// yet; that command must consume this factory when it lands.
    #[allow(dead_code)]
    pub fn comfy_mesh(&self) -> Result<ComfyMeshBackend, ImageError> {
        ComfyMeshBackend::new(self.comfyui_endpoint())
    }
}

#[tauri::command]
pub fn machine_settings(settings: State<'_, MachineSettings>) -> MachineSettingsView {
    settings.view()
}

/// What the shell needs before it draws its first surface: whether the legal
/// gate has been passed, and whether the tour has been seen.
#[tauri::command]
pub fn onboarding_state(settings: State<'_, MachineSettings>) -> OnboardingState {
    settings.onboarding()
}

/// The acceptance step of the first run.
///
/// Separate from [`onboarding_finish`] on purpose. Skipping the tour is a
/// preference; accepting the documents is not, and collapsing the two into one
/// "onboarding done" flag would make a dismissed overlay indistinguishable from
/// an agreement. Nothing un-accepts: re-running the tour re-opens the overlay
/// but leaves the recorded acceptance alone.
#[tauri::command]
pub fn onboarding_accept_legal(
    settings: State<'_, MachineSettings>,
    version: String,
) -> CommandResult<OnboardingState> {
    settings.accept_legal(version)
}

/// Finished or skipped — both mean the overlay should not open itself again.
#[tauri::command]
pub fn onboarding_finish(settings: State<'_, MachineSettings>) -> CommandResult<OnboardingState> {
    settings.finish_onboarding()
}

#[tauri::command]
pub fn comfyui_endpoint_set(
    settings: State<'_, MachineSettings>,
    endpoint: String,
) -> CommandResult<MachineSettingsView> {
    settings.set_comfyui_endpoint(&endpoint)
}

#[tauri::command]
pub async fn comfyui_endpoint_probe(
    settings: State<'_, MachineSettings>,
    endpoint: Option<String>,
) -> CommandResult<ComfyEndpointProbe> {
    let endpoint = match endpoint {
        Some(endpoint) => normalise_endpoint(&endpoint)?,
        None => settings.comfyui_endpoint(),
    };
    match ComfyBackend::connect(&endpoint).await {
        Ok(backend) => {
            let installed = backend.installed().unwrap_or_default();
            let images = installed.checkpoints().len() + installed.unets().len();
            let meshes = installed.mesh_models().len();
            Ok(ComfyEndpointProbe {
                endpoint,
                state: ComfyEndpointState::Connected,
                ok: true,
                message: format!(
                    "Connected to ComfyUI. Found {images} image model{} and {meshes} local mesh model{}.",
                    plural(images),
                    plural(meshes),
                ),
            })
        }
        Err(error) => {
            let detail = error.to_string();
            let state = classify_probe(&error);
            Ok(ComfyEndpointProbe { endpoint, state, ok: false, message: detail })
        }
    }
}

fn classify_probe(error: &ImageError) -> ComfyEndpointState {
    let detail = error.to_string();
    if detail.contains("requires authentication") {
        ComfyEndpointState::AuthenticationRequired
    } else if detail.contains("it is not ComfyUI") {
        ComfyEndpointState::Incompatible
    } else {
        ComfyEndpointState::Unreachable
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn normalise_endpoint(raw: &str) -> CommandResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(comfy::DEFAULT_URL.to_owned());
    }
    let candidate =
        if trimmed.contains("://") { trimmed.to_owned() } else { format!("http://{trimmed}") };
    let parsed = Url::parse(&candidate).map_err(|error| {
        WobuError::new(Code::Invalid, "That is not a valid ComfyUI endpoint.")
            .with_detail(error.to_string())
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(WobuError::new(
            Code::Invalid,
            "A ComfyUI endpoint must use http:// or https://.",
        ));
    }
    if parsed.host_str().is_none() {
        return Err(WobuError::new(Code::Invalid, "A ComfyUI endpoint needs a host name."));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(WobuError::new(
            Code::Invalid,
            "Do not put a username or password in the ComfyUI URL. Wobu does not persist endpoint credentials.",
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(WobuError::new(
            Code::Invalid,
            "The ComfyUI endpoint cannot contain a query string or fragment.",
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn save(path: &Path, stored: &Stored) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| io_error("create", path, error))?;
    }
    let json = serde_json::to_string_pretty(stored).expect("machine settings contain one string");
    std::fs::write(path, json).map_err(|error| io_error("write", path, error))?;
    paths::restrict(path).map_err(|error| io_error("protect", path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, format!("Could not {action} this computer's Wobu settings."))
        .with_detail(format!("{}: {error}", path.display()))
}

fn default_path() -> PathBuf {
    paths::app_data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wobu-machine-{}-{name}-{n}.json", std::process::id()))
    }

    #[test]
    fn endpoint_survives_restart_outside_any_project() {
        let path = scratch("restart");
        let settings = MachineSettings::load_from(path.clone());
        assert_eq!(settings.comfyui_endpoint(), comfy::DEFAULT_URL);
        settings.set_comfyui_endpoint(" HTTPS://renderbox.example:9443/comfy/ ").unwrap();

        let reloaded = MachineSettings::load_from(path.clone());
        assert_eq!(reloaded.comfyui_endpoint(), "https://renderbox.example:9443/comfy");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("comfyuiEndpoint"));
        assert!(!raw.contains("project"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_or_credential_bearing_urls_are_never_persisted() {
        let path = scratch("validation");
        let settings = MachineSettings::load_from(path.clone());
        settings.set_comfyui_endpoint("renderbox:8189").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();

        for invalid in [
            "file:///tmp/comfy",
            "http://user:secret@renderbox:8189",
            "http://renderbox:8189?token=secret",
            "http://renderbox:8189/#secret",
        ] {
            assert!(settings.set_comfyui_endpoint(invalid).is_err(), "accepted {invalid}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn image_status_generation_composition_and_mesh_share_one_route() {
        let path = scratch("routing");
        let settings = MachineSettings::load_from(path.clone());
        settings.set_comfyui_endpoint("renderbox.local:9000/proxy").unwrap();

        assert_eq!(settings.comfy_image().unwrap().base_url(), "http://renderbox.local:9000/proxy");
        assert_eq!(settings.comfy_mesh().unwrap().base_url(), "http://renderbox.local:9000/proxy");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_installation_that_predates_onboarding_has_accepted_nothing() {
        let path = scratch("legacy");
        std::fs::write(&path, r#"{"comfyuiEndpoint":"http://renderbox:8188"}"#).unwrap();

        let settings = MachineSettings::load_from(path.clone());
        assert_eq!(settings.onboarding(), OnboardingState::default());
        assert_eq!(settings.comfyui_endpoint(), "http://renderbox:8188");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn acceptance_outlives_a_restart_and_an_endpoint_change() {
        let path = scratch("acceptance");
        let settings = MachineSettings::load_from(path.clone());
        settings.accept_legal("terms 3 August 2026".into()).unwrap();
        // The regression this pins: writing one field must not rewrite the file
        // without the others.
        settings.set_comfyui_endpoint("renderbox.local:8188").unwrap();

        let reloaded = MachineSettings::load_from(path.clone());
        let state = reloaded.onboarding();
        assert_eq!(state.legal_version.as_deref(), Some("terms 3 August 2026"));
        assert!(state.legal_accepted_at.is_some());
        assert!(state.completed_at.is_none(), "accepting is not finishing the tour");
        assert_eq!(reloaded.comfyui_endpoint(), "http://renderbox.local:8188");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn finishing_the_tour_leaves_the_recorded_agreement_alone() {
        let path = scratch("finish");
        let settings = MachineSettings::load_from(path.clone());
        settings.accept_legal("v1".into()).unwrap();
        let accepted = settings.onboarding().legal_accepted_at;

        let after = settings.finish_onboarding().unwrap();
        assert!(after.completed_at.is_some());
        assert_eq!(after.legal_accepted_at, accepted);
        assert_eq!(after.legal_version.as_deref(), Some("v1"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_endpoint_this_build_rejects_does_not_discard_the_agreement() {
        let path = scratch("salvage");
        let settings = MachineSettings::load_from(path.clone());
        settings.accept_legal("v1".into()).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, raw.replace(comfy::DEFAULT_URL, "file:///tmp/comfy")).unwrap();

        let reloaded = MachineSettings::load_from(path.clone());
        assert_eq!(reloaded.comfyui_endpoint(), comfy::DEFAULT_URL);
        assert_eq!(reloaded.onboarding().legal_version.as_deref(), Some("v1"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn settings_file_is_in_application_data_not_a_world() {
        assert_eq!(default_path(), paths::app_data_dir().join("settings.json"));
    }

    #[test]
    fn authentication_and_schema_failures_remain_distinct_from_reachability() {
        let auth = ImageError::Unavailable {
            detail: "the endpoint requires authentication (HTTP 401)".into(),
        };
        let schema =
            ImageError::Unavailable { detail: "something answered, but it is not ComfyUI".into() };
        let down = ImageError::Unavailable { detail: "nothing is listening".into() };
        assert_eq!(classify_probe(&auth), ComfyEndpointState::AuthenticationRequired);
        assert_eq!(classify_probe(&schema), ComfyEndpointState::Incompatible);
        assert_eq!(classify_probe(&down), ComfyEndpointState::Unreachable);
    }
}
