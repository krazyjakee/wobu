//! Validation and publication of immutable project-owned LoRA weights.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::{atomic, paths};

const MAX_HEADER_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_WEIGHT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Validate the safetensors envelope without deserialising tensor data.
pub fn validate(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 9 || bytes.len() as u64 > MAX_WEIGHT_BYTES {
        return Err(Error::InvalidLora("the file is empty, truncated, or over 2 GB".into()));
    }
    let header_len = u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes checked"));
    let header_len = usize::try_from(header_len)
        .ok()
        .filter(|length| *length > 0 && *length <= MAX_HEADER_BYTES)
        .ok_or_else(|| Error::InvalidLora("the header length is invalid".into()))?;
    let data_start = 8usize
        .checked_add(header_len)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| Error::InvalidLora("the header extends beyond the file".into()))?;
    let header: Value = serde_json::from_slice(&bytes[8..data_start])
        .map_err(|error| Error::InvalidLora(format!("the header is not JSON ({error})")))?;
    let entries = header
        .as_object()
        .ok_or_else(|| Error::InvalidLora("the header is not an object".into()))?;
    let data_len = bytes.len() - data_start;
    let mut tensors = 0usize;
    for (name, value) in entries {
        if name == "__metadata__" {
            continue;
        }
        let tensor = value
            .as_object()
            .ok_or_else(|| Error::InvalidLora(format!("tensor {name} has no descriptor")))?;
        let offsets = tensor
            .get("data_offsets")
            .and_then(Value::as_array)
            .filter(|offsets| offsets.len() == 2)
            .ok_or_else(|| Error::InvalidLora(format!("tensor {name} has invalid offsets")))?;
        let start = offsets[0].as_u64().and_then(|value| usize::try_from(value).ok());
        let end = offsets[1].as_u64().and_then(|value| usize::try_from(value).ok());
        let (Some(start), Some(end)) = (start, end) else {
            return Err(Error::InvalidLora(format!("tensor {name} has non-integer offsets")));
        };
        if start > end || end > data_len {
            return Err(Error::InvalidLora(format!("tensor {name} points outside the file")));
        }
        if tensor.get("dtype").and_then(Value::as_str).is_none()
            || tensor.get("shape").and_then(Value::as_array).is_none()
        {
            return Err(Error::InvalidLora(format!("tensor {name} has no dtype or shape")));
        }
        tensors += 1;
    }
    if tensors == 0 {
        return Err(Error::InvalidLora("the file contains no tensors".into()));
    }
    Ok(())
}

/// Publish validated bytes at their hash-derived path. Existing identical
/// bytes are a dedupe; an existing mismatched path is corruption.
pub fn publish(root: &Path, hash: &str, bytes: &[u8]) -> Result<(PathBuf, bool)> {
    validate(bytes)?;
    if atomic::hash_bytes(bytes) != hash {
        return Err(Error::InvalidLora("the declared content hash does not match".into()));
    }
    let rel = wobu_core::asset::lora_path(hash).ok_or_else(|| {
        Error::InvalidLora("the content hash is not 64 lowercase hex characters".into())
    })?;
    let target = paths::from_rel_string(root, &rel);
    if let Ok(metadata) = std::fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(Error::InvalidLora(
                "the content-addressed destination is not a regular file".into(),
            ));
        }
        let existing = std::fs::read(&target).map_err(|error| Error::io(&target, error))?;
        if atomic::hash_bytes(&existing) != hash {
            return Err(Error::InvalidLora("the content-addressed destination is corrupt".into()));
        }
        return Ok((target, true));
    }
    match atomic::write_once(root, &target, bytes) {
        Ok(_) => Ok((target, false)),
        Err(Error::AlreadyExists(_)) => {
            let metadata =
                std::fs::symlink_metadata(&target).map_err(|error| Error::io(&target, error))?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(Error::InvalidLora(
                    "the content-addressed destination is not a regular file".into(),
                ));
            }
            let existing = std::fs::read(&target).map_err(|error| Error::io(&target, error))?;
            if atomic::hash_bytes(&existing) == hash {
                Ok((target, true))
            } else {
                Err(Error::InvalidLora("the content-addressed destination is corrupt".into()))
            }
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weight() -> Vec<u8> {
        let header = br#"{"character.lora":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#;
        let mut bytes = Vec::with_capacity(8 + header.len() + 4);
        bytes.extend_from_slice(&(header.len() as u64).to_le_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes
    }

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("wobu-lora-test-{}", wobu_core::new_id()));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn validates_and_deduplicates_a_content_addressed_safetensors_weight() {
        let bytes = weight();
        validate(&bytes).unwrap();
        let hash = atomic::hash_bytes(&bytes);
        let root = root();
        let (path, deduped) = publish(&root, &hash, &bytes).unwrap();
        assert!(!deduped);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(publish(&root, &hash, &bytes).unwrap().1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_truncated_tensor_data_and_a_false_declared_hash() {
        let mut bytes = weight();
        bytes.pop();
        assert!(matches!(validate(&bytes), Err(Error::InvalidLora(_))));
        let root = root();
        assert!(matches!(publish(&root, &"0".repeat(64), &weight()), Err(Error::InvalidLora(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_at_the_content_addressed_destination() {
        use std::os::unix::fs::symlink;

        let bytes = weight();
        let hash = atomic::hash_bytes(&bytes);
        let root = root();
        let rel = wobu_core::asset::lora_path(&hash).unwrap();
        let target = root.join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let outside = root.join("outside.safetensors");
        std::fs::write(&outside, &bytes).unwrap();
        symlink(&outside, &target).unwrap();
        assert!(matches!(publish(&root, &hash, &bytes), Err(Error::InvalidLora(_))));
        let _ = std::fs::remove_dir_all(root);
    }
}
