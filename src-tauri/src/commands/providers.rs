//! Which provider a capability is set to, and which model that means.
//!
//! One reader for the `providers` map in `project.json`. There used to be
//! three, one per caller: image generation, the status bar and 3D
//! reconstruction each re-implemented "read the object, take `provider`, trim
//! it, reject it if empty, then take `model` the same way". Three copies of a
//! rule this small drift by omission rather than by disagreement — the mesh
//! copy, for instance, is the only one that also had to read `region` — so the
//! rule lives here and the differences are arguments.
//!
//! Model resolution is deliberately part of the same type. "Which model" is
//! only half a project decision: the project may pin one, the caller may
//! override it for one request, and the adapter supplies the default when
//! neither did. Splitting those three across call sites is how a request ends
//! up naming a model the receipt then disagrees with.

use serde_json::{Map, Value};
use wobu_store::Project;

/// A capability's selected provider, exactly as `project.json` records it.
#[derive(Debug, Clone)]
pub struct ProviderChoice {
    pub provider: String,
    /// The model this project pinned, if it pinned one. `None` means "whatever
    /// this adapter's default is", which only the adapter can answer.
    pub configured_model: Option<String>,
    settings: Map<String, Value>,
}

impl ProviderChoice {
    /// The selection for `capability`, or `None` when the project names no
    /// usable provider for it. Callers decide whether that is an error: it is
    /// for generation, and it is an ordinary empty state for the status bar.
    pub fn of(project: &Project, capability: &str) -> Option<ProviderChoice> {
        let settings = project.meta().providers.get(capability)?.as_object()?;
        let provider = text(settings, "provider")?;
        Some(ProviderChoice {
            provider,
            configured_model: text(settings, "model"),
            settings: settings.clone(),
        })
    }

    /// The model to send: the caller's override, else the project's pin, else
    /// the adapter's own default.
    pub fn model(&self, requested: Option<String>, backend_default: &str) -> String {
        requested
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| self.configured_model.clone())
            .unwrap_or_else(|| backend_default.to_owned())
    }

    /// Any other non-empty string the project recorded beside the provider —
    /// Hunyuan3D's `region`, for instance. Provider-specific by definition, so
    /// it is read by key rather than promoted to a field every capability would
    /// then carry and ignore.
    pub fn setting(&self, key: &str) -> Option<String> {
        text(&self.settings, key)
    }
}

fn text(settings: &Map<String, Value>, key: &str) -> Option<String> {
    settings
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(settings: Value) -> Option<ProviderChoice> {
        let settings = settings.as_object().unwrap().clone();
        let provider = text(&settings, "provider")?;
        Some(ProviderChoice { provider, configured_model: text(&settings, "model"), settings })
    }

    #[test]
    fn blank_and_whitespace_only_fields_are_not_selections() {
        assert!(choice(serde_json::json!({ "provider": "   " })).is_none());
        assert!(choice(serde_json::json!({ "model": "gemini-3.1-flash-image" })).is_none());
        let trimmed = choice(serde_json::json!({ "provider": " gemini ", "model": "  " })).unwrap();
        assert_eq!(trimmed.provider, "gemini");
        assert_eq!(trimmed.configured_model, None);
    }

    #[test]
    fn the_request_beats_the_project_and_the_project_beats_the_adapter() {
        let pinned =
            choice(serde_json::json!({ "provider": "gemini", "model": "pinned" })).unwrap();
        assert_eq!(pinned.model(Some(" asked ".into()), "default"), "asked");
        assert_eq!(pinned.model(Some("   ".into()), "default"), "pinned");
        assert_eq!(pinned.model(None, "default"), "pinned");

        let unpinned = choice(serde_json::json!({ "provider": "gemini" })).unwrap();
        assert_eq!(unpinned.model(None, "default"), "default");
    }

    #[test]
    fn provider_specific_settings_are_read_by_key() {
        let hunyuan =
            choice(serde_json::json!({ "provider": "tencent", "region": " ap-singapore " }))
                .unwrap();
        assert_eq!(hunyuan.setting("region").as_deref(), Some("ap-singapore"));
        assert_eq!(hunyuan.setting("endpoint"), None);
    }
}
