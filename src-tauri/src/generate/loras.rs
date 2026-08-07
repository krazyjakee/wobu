//! Which LoRAs a generation runs with, and what its receipt may claim.
//!
//! A LoRA is named in the project but lives on disk beside the local backend,
//! so the set a request can use is the intersection of what the node asks for
//! and what the backend admits. Whatever is dropped is reported rather than
//! silently omitted: a receipt that recorded four LoRAs when three ran would
//! make the image unreproducible.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use wobu_core::{Id, Node};
use wobu_imagine::{ImageBackend, LoraWeight};

const LORA_PROTOCOL: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReceiptLora {
    pub(super) node_id: Id,
    pub(super) content_hash: String,
    pub(super) provider_name: String,
    pub(super) trigger_token: String,
    pub(super) strength: f32,
}

impl ReceiptLora {
    pub(super) fn weight(&self) -> LoraWeight {
        LoraWeight {
            content_hash: self.content_hash.clone(),
            provider_name: self.provider_name.clone(),
            trigger_token: self.trigger_token.clone(),
            strength: self.strength,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoraDowngrade {
    pub(super) node_id: Id,
    pub(super) content_hash: String,
    pub(super) state: &'static str,
    pub(super) detail: String,
}

#[derive(Clone)]
pub(super) struct ResolvedLoras {
    pub(super) receipts: Vec<ReceiptLora>,
    pub(super) weights: Vec<LoraWeight>,
    pub(super) downgrades: Vec<LoraDowngrade>,
}

pub(super) fn resolve_loras(
    root: &Path,
    nodes: &[Node],
    ordered_node_ids: impl IntoIterator<Item = Id>,
    model: &str,
    backend: &dyn ImageBackend,
) -> ResolvedLoras {
    let by_id: HashMap<Id, &Node> = nodes.iter().map(|node| (node.id, node)).collect();
    let mut visited_nodes = HashSet::new();
    let mut applied_hashes = HashSet::new();
    let mut receipts = Vec::new();
    let mut weights = Vec::new();
    let mut downgrades = Vec::new();
    for node_id in ordered_node_ids {
        if !visited_nodes.insert(node_id) {
            continue;
        }
        let Some(pin) = by_id.get(&node_id).and_then(|node| node.lora.as_ref()) else {
            continue;
        };
        let reject = |state: &'static str, detail: String| LoraDowngrade {
            node_id,
            content_hash: pin.hash.clone(),
            state,
            detail,
        };
        if pin.protocol != LORA_PROTOCOL {
            downgrades.push(reject(
                "protocol_mismatch",
                format!("The pin uses trainer protocol {}, not {LORA_PROTOCOL}.", pin.protocol),
            ));
            continue;
        }
        if pin.base_model != model {
            downgrades.push(reject(
                "model_mismatch",
                format!("The LoRA was trained for {}, not {model}.", pin.base_model),
            ));
            continue;
        }
        if !pin.strength.is_finite() || !(0.0..=2.0).contains(&pin.strength) {
            downgrades.push(reject("weight_corrupt", "The LoRA strength is invalid.".into()));
            continue;
        }
        if !safe_trigger_token(&pin.trigger_token) || !safe_lora_name(&pin.provider_name) {
            downgrades.push(reject(
                "pin_invalid",
                "The LoRA pin contains an unsafe trigger token or provider filename.".into(),
            ));
            continue;
        }
        let Some(expected_path) = wobu_core::asset::lora_path(&pin.hash) else {
            downgrades.push(reject("pin_invalid", "The LoRA content hash is invalid.".into()));
            continue;
        };
        if pin.rel_path != expected_path {
            downgrades.push(reject(
                "pin_invalid",
                "The LoRA path does not match its content hash.".into(),
            ));
            continue;
        }
        let path = root.join(expected_path);
        let valid_bytes = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
            .filter(|metadata| metadata.len() == pin.bytes)
            .and_then(|_| std::fs::read(&path).ok())
            .filter(|bytes| wobu_store::atomic::hash_bytes(bytes) == pin.hash)
            .filter(|bytes| wobu_store::lora::validate(bytes).is_ok());
        if valid_bytes.is_none() {
            downgrades.push(reject(
                "weight_missing_or_corrupt",
                "The project-owned LoRA is missing or failed its integrity check.".into(),
            ));
            continue;
        }
        if !backend.supports_lora(model, &pin.provider_name) {
            downgrades.push(reject(
                "provider_unsupported",
                "The probed provider cannot load this LoRA for the selected model.".into(),
            ));
            continue;
        }
        if !applied_hashes.insert(pin.hash.clone()) {
            downgrades.push(reject(
                "deduplicated",
                "An earlier influence source already applies the same content-addressed LoRA."
                    .into(),
            ));
            continue;
        }
        let receipt = ReceiptLora {
            node_id,
            content_hash: pin.hash.clone(),
            provider_name: pin.provider_name.clone(),
            trigger_token: pin.trigger_token.clone(),
            strength: pin.strength,
        };
        weights.push(receipt.weight());
        receipts.push(receipt);
    }
    ResolvedLoras { receipts, weights, downgrades }
}

pub(super) fn prompt_with_lora_triggers(prompt: &str, loras: &[LoraWeight]) -> String {
    let triggers = missing_lora_triggers(prompt, loras);
    if triggers.is_empty() {
        prompt.to_owned()
    } else if prompt.trim().is_empty() {
        triggers.join(", ")
    } else {
        format!("{}, {}", prompt.trim_end(), triggers.join(", "))
    }
}

pub(super) fn scene_prompt_with_lora_triggers(prompt: &str, loras: &[LoraWeight]) -> String {
    let triggers = missing_lora_triggers(prompt, loras);
    if triggers.is_empty() {
        return prompt.to_owned();
    }
    let trigger_clause = triggers.join(", ");
    match prompt.rsplit_once("; ") {
        Some((before_identity, identity)) => {
            format!("{before_identity}; {trigger_clause}; {identity}")
        }
        None if prompt.trim().is_empty() => trigger_clause,
        None => format!("{trigger_clause}; {prompt}"),
    }
}

fn missing_lora_triggers<'a>(prompt: &str, loras: &'a [LoraWeight]) -> Vec<&'a str> {
    let existing: HashSet<&str> = prompt
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '_' || character == '-')
        })
        .filter(|part| !part.is_empty())
        .collect();
    let mut seen = HashSet::new();
    loras
        .iter()
        .map(|lora| lora.trigger_token.as_str())
        .filter(|token| !existing.contains(token) && seen.insert(*token))
        .collect()
}

pub(super) fn safe_trigger_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(super) fn safe_lora_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name.ends_with(".safetensors")
        && !name.starts_with('/')
        && !name.contains('\\')
        && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}
