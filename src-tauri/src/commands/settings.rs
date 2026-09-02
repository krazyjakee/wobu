//! Which provider each capability is set to, and what that resolves to now.
//!
//! The selection is written to `project.json` so a shared project agrees about
//! it; the *health* of that selection is not, because it depends on a key in
//! this machine's keychain and a service that may be down. `status_bar_backend`
//! is the one command that answers both halves at once.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::State;
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_imagine::{comfy, gemini as image_gemini, tencent::Region as HunyuanRegion};
use wobu_llm::{Cancel, anthropic, gemini};
use wobu_store::Project;

use super::providers::ProviderChoice;
use crate::error::{Code, CommandResult, WobuError};
use crate::keys::Keys;
use crate::machine::MachineSettings;
use crate::state::AppState;

/// Which job a selection is for.
///
/// Three, not one list. A user enhancing with Gemini, generating on a ComfyUI
/// running on the machine under their desk and meshing through Hunyuan3D is the
/// ordinary case rather than the exotic one (`docs/08-providers.md`), and a
/// single "provider" setting would make it unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Text,
    Image,
    Mesh,
}

impl Capability {
    /// The key this capability's selection sits under in `project.json`'s
    /// `providers`.
    ///
    /// `"text"` is not a name chosen here: `enhance.rs` already reads that key
    /// to decide who writes descriptions, and the two have to agree or a
    /// selection made in Settings is written somewhere Enhance never looks.
    pub(super) fn key(self) -> &'static str {
        match self {
            Capability::Text => "text",
            Capability::Image => "image",
            Capability::Mesh => "mesh",
        }
    }
}

/// The shared half of the providers pane: what `project.json` says.
///
/// Carried as the raw map rather than as three typed fields, because a project
/// written by a build that knows a fourth capability must survive a round trip
/// through this one. The frontend reads the capabilities it understands and
/// leaves the rest alone, which is the same contract `ProjectMeta` has with the
/// file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSelections {
    pub providers: serde_json::Map<String, serde_json::Value>,
    /// Whether the *selection* can be changed here. Keys are unaffected: they
    /// are per installation and go to this machine's keychain, so a read-only
    /// world is one you can still add a key for and still generate from.
    pub read_only: bool,
}

/// What this project has chosen, for the pane that has to show it as shared.
#[tauri::command]
pub fn project_providers(state: State<'_, AppState>) -> CommandResult<ProviderSelections> {
    state.with(|p| Ok(selections(p)))
}

/// Choose a provider for one capability, and write it into `project.json`.
///
/// This is the only command that writes project metadata, and it exists because
/// there was no other: `wobu-store` writes `project.json` when a project is
/// created and never again. Anything else that needs to change that file should
/// come through here rather than open a second way to do it.
///
/// The selection is *shared*. It travels with the folder to everyone on the
/// share, which is exactly why the key does not — see `keys.rs`.
#[tauri::command]
pub fn project_provider_select(
    state: State<'_, AppState>,
    capability: Capability,
    provider: String,
    model: Option<String>,
    region: Option<String>,
) -> CommandResult<ProviderSelections> {
    let provider = provider.trim().to_owned();
    if provider.is_empty() {
        return Err(WobuError::new(Code::Invalid, "A capability needs a provider."));
    }
    // Trimmed to nothing means "whatever the adapter's default is", which is a
    // real answer and is spelled as the absence of the field — the same thing
    // `enhance.rs` reads an empty string as.
    let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    let region = provider_region(capability, &provider, region)?;

    state.with(|project| {
        if project.is_read_only() {
            return Err(WobuError::new(
                Code::ReadOnly,
                "This project folder is read-only, so the provider it uses cannot be changed \
                 here. Keys can still be added — those live on this machine.",
            ));
        }

        let root = project.root().to_path_buf();
        let mut providers = project.meta().providers.clone();
        // Merged into whatever is already under this capability rather than
        // replacing it. Default params live in the same object
        // (`docs/08-providers.md`), and a build that only knows about `provider`
        // and `model` must not delete the rest of somebody's settings by
        // touching a dropdown.
        let mut chosen = providers
            .get(capability.key())
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        chosen.insert("provider".to_owned(), serde_json::Value::String(provider));
        match model {
            Some(model) => {
                chosen.insert("model".to_owned(), serde_json::Value::String(model));
            }
            None => {
                chosen.remove("model");
            }
        }
        // Omitted means "leave the existing region alone": provider buttons
        // and model edits do not silently move an existing project between
        // data-processing regions. A value only comes from the explicit
        // Hunyuan region picker above.
        if let Some(region) = region {
            chosen.insert("region".to_owned(), serde_json::Value::String(region));
        }
        providers.insert(capability.key().to_owned(), serde_json::Value::Object(chosen));
        write_providers(&root, &providers)?;

        // Reopened rather than patched in memory, because `Project` hands out
        // `&ProjectMeta` and nothing else — `meta` is what was read at open
        // time. Without this the user would change the provider, press Enhance,
        // and be billed by the one they just moved away from, which is the
        // exact failure `enhance.rs`'s selection code is written to prevent.
        //
        // It costs a `reconcile` — the same walk the Reload button does — and
        // this runs when somebody picks from a dropdown, not in a loop.
        *project = Project::open(&root)?;
        Ok(selections(project))
    })
}

pub(super) fn provider_region(
    capability: Capability,
    provider: &str,
    region: Option<String>,
) -> CommandResult<Option<String>> {
    let region = region.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    if let Some(region) = &region
        && (capability != Capability::Mesh
            || provider != "hunyuan3d"
            || HunyuanRegion::parse(region).is_none())
    {
        return Err(WobuError::new(
            Code::Invalid,
            "Tencent Hunyuan3D region must be ap-singapore, na-siliconvalley or eu-frankfurt.",
        ));
    }
    Ok(region)
}

fn selections(project: &Project) -> ProviderSelections {
    ProviderSelections {
        providers: project.meta().providers.clone(),
        read_only: project.is_read_only(),
    }
}

/* ── status-bar provider health ──────────────────────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveModel {
    pub provider: String,
    pub label: String,
    pub model: String,
    /// Known window for a shipped text model. Unknown custom model ids stay
    /// `None`; guessing a context window would make the status bar dangerous.
    pub context_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum BackendHealth {
    Connected { external_queue: Option<u32> },
    Unavailable { detail: String },
    Unconfigured { detail: String },
    Unsupported { detail: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusBarBackend {
    pub image: Option<ActiveModel>,
    pub text: ActiveModel,
    pub health: BackendHealth,
}

/// The provider facts the status bar can defend.
///
/// The project lock is released before the reachability request. Holding it
/// across a network call would freeze every editor read behind a health check.
#[tauri::command]
pub async fn status_bar_backend(
    state: State<'_, AppState>,
    keys: State<'_, Keys>,
    machine: State<'_, MachineSettings>,
) -> CommandResult<StatusBarBackend> {
    let (image, text) = state.with(|project| {
        Ok((ProviderChoice::of(project, "image"), ProviderChoice::of(project, "text")))
    })?;

    let text_provider = text.as_ref().map_or(anthropic::ID, |choice| choice.provider.as_str());
    let text_model = text
        .as_ref()
        .and_then(|choice| choice.configured_model.clone())
        .unwrap_or_else(|| text_default(text_provider).to_owned());
    let text = ActiveModel {
        provider: text_provider.to_owned(),
        label: provider_label(text_provider).to_owned(),
        context_tokens: context_window(text_provider, &text_model),
        model: text_model,
    };

    let Some(image) = image else {
        return Ok(StatusBarBackend {
            image: None,
            text,
            health: BackendHealth::Unconfigured {
                detail: "No image backend is selected for this project.".into(),
            },
        });
    };

    let image_provider = image.provider.clone();
    let image_model = image.model(None, image_default(&image_provider));
    let image = ActiveModel {
        provider: image_provider.clone(),
        label: provider_label(&image_provider).to_owned(),
        model: image_model.clone(),
        context_tokens: None,
    };

    let health = match image_provider.as_str() {
        comfy::ID => match machine.comfy_image() {
            Ok(backend) => match backend.health(&image_model).await {
                comfy::Health::Connected { queue, .. } => {
                    BackendHealth::Connected { external_queue: Some(queue) }
                }
                comfy::Health::Unreachable { detail } => BackendHealth::Unavailable { detail },
            },
            Err(error) => BackendHealth::Unavailable { detail: error.to_string() },
        },
        image_gemini::ID => match keys.secret(image_gemini::ID).await? {
            None => BackendHealth::Unconfigured {
                detail: "Gemini is selected for images, but this machine has no Gemini key.".into(),
            },
            Some(secret) => match image_gemini::GeminiBackend::new(secret.expose()) {
                Ok(backend) => match backend.check_key(&image_model, &Cancel::new()).await {
                    image_gemini::KeyCheck::Usable => {
                        BackendHealth::Connected { external_queue: None }
                    }
                    check => BackendHealth::Unavailable { detail: check.message() },
                },
                Err(error) => BackendHealth::Unavailable { detail: error.to_string() },
            },
        },
        other => BackendHealth::Unsupported {
            detail: format!("This build has no image adapter for {other}."),
        },
    };

    Ok(StatusBarBackend { image: Some(image), text, health })
}

fn provider_label(provider: &str) -> &str {
    match provider {
        anthropic::ID => anthropic::LABEL,
        gemini::ID => gemini::LABEL,
        comfy::ID => comfy::LABEL,
        _ => provider,
    }
}

pub(super) fn text_default(provider: &str) -> &str {
    match provider {
        anthropic::ID => anthropic::DEFAULT_MODEL,
        gemini::ID => gemini::DEFAULT_MODEL,
        _ => "unknown provider default",
    }
}

pub(super) fn image_default(provider: &str) -> &str {
    match provider {
        comfy::ID => comfy::DEFAULT_MODEL,
        image_gemini::ID => image_gemini::DEFAULT_MODEL,
        _ => "unknown provider default",
    }
}

pub(super) fn context_window(provider: &str, model: &str) -> Option<u64> {
    match (provider, model) {
        (anthropic::ID, "claude-haiku-4-5") => Some(200_000),
        (anthropic::ID, "claude-opus-5" | "claude-sonnet-5" | "claude-fable-5") => Some(1_000_000),
        (gemini::ID, "gemini-3.6-flash") => Some(1_048_576),
        (gemini::ID, "gemini-3.5-flash" | "gemini-3.5-flash-lite") => Some(1_000_000),
        _ => None,
    }
}

/// Put `providers` into `project.json`, leaving every other byte of meaning
/// alone.
///
/// Read back as raw JSON and patched at one key rather than re-serialised from
/// `ProjectMeta`: a field written by a newer Wobu would not survive a round trip
/// through a struct that has never heard of it, and `project.json` is precisely
/// the file two builds of different vintages share across a drive.
///
/// Staged and renamed, on the same filesystem so the rename is atomic. This is
/// the file that decides whether a folder is a project at all — a half-written
/// one is a world that will not open, for everyone on the share at once.
pub(super) fn write_providers(
    root: &Path,
    providers: &serde_json::Map<String, serde_json::Value>,
) -> CommandResult<()> {
    let path = root.join("project.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| meta_write_failed("could not be read", e.to_string()))?;
    let mut meta: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| meta_write_failed("could not be read", e.to_string()))?;
    let Some(object) = meta.as_object_mut() else {
        return Err(meta_write_failed("is not a JSON object", raw));
    };
    object.insert("providers".to_owned(), serde_json::Value::Object(providers.clone()));

    let staging = root.join(".wobu").join("tmp");
    std::fs::create_dir_all(&staging)
        .map_err(|e| meta_write_failed("could not be staged", e.to_string()))?;
    let part = staging.join("project.json.part");
    let text = serde_json::to_string_pretty(&meta)
        .map_err(|e| meta_write_failed("could not be written", e.to_string()))?;
    std::fs::write(&part, text)
        .map_err(|e| meta_write_failed("could not be staged", e.to_string()))?;
    std::fs::rename(&part, &path)
        .map_err(|e| meta_write_failed("could not be written", e.to_string()))?;
    Ok(())
}

fn meta_write_failed(what_happened: &str, detail: String) -> WobuError {
    WobuError::new(Code::Io, format!("This project's `project.json` {what_happened}."))
        .with_detail(detail)
}

/* ── the capability probe ─────────────────────────────────────────────────── */
