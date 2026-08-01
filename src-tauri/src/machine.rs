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
}

impl Default for Stored {
    fn default() -> Self {
        Self { comfyui_endpoint: comfy::DEFAULT_URL.to_owned() }
    }
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
            .and_then(|stored| {
                normalise_endpoint(&stored.comfyui_endpoint)
                    .ok()
                    .map(|comfyui_endpoint| Stored { comfyui_endpoint })
            })
            .unwrap_or_default();
        Self { path, stored: RwLock::new(stored) }
    }

    pub fn view(&self) -> MachineSettingsView {
        MachineSettingsView { comfyui_endpoint: self.comfyui_endpoint() }
    }

    pub fn comfyui_endpoint(&self) -> String {
        self.stored.read().comfyui_endpoint.clone()
    }

    pub fn set_comfyui_endpoint(&self, endpoint: &str) -> CommandResult<MachineSettingsView> {
        let endpoint = normalise_endpoint(endpoint)?;
        let next = Stored { comfyui_endpoint: endpoint };
        save(&self.path, &next)?;
        *self.stored.write() = next;
        Ok(self.view())
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
    restrict(path).map_err(|error| io_error("protect", path, error))
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> WobuError {
    WobuError::new(Code::Io, format!("Could not {action} this computer's Wobu settings."))
        .with_detail(format!("{}: {error}", path.display()))
}

fn default_path() -> PathBuf {
    paths::app_data_dir().join("settings.json")
}

#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
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
