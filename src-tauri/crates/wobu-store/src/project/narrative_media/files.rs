use super::*;
use crate::atomic;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub(super) fn safe_path(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.len() > 240
        || !rel.is_ascii()
        || rel.bytes().any(|c| c.is_ascii_control() || b"\\:*?\"<>|".contains(&c))
        || rel.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.ends_with(['.', ' ']))
    {
        return Err(invalid(
            "Audio/timing paths must be portable relative paths without traversal.",
        ));
    }
    let mut path = root.to_path_buf();
    for segment in rel.split('/') {
        let stem = segment.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(invalid("Reserved device path."));
        }
        path.push(segment);
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            return Err(invalid("Media paths cannot traverse symbolic links."));
        }
    }
    Ok(path)
}
pub(super) fn read_limited(root: &Path, rel: &str, max: usize) -> Result<Vec<u8>> {
    let path = safe_path(root, rel)?;
    let meta = std::fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
    if !meta.is_file() || meta.len() > max as u64 {
        return Err(invalid("Media input must be a bounded regular file."));
    }
    let file = std::fs::File::open(&path).map_err(|e| Error::io(&path, e))?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|e| Error::io(&path, e))?;
    if bytes.len() > max {
        return Err(invalid("Media input grew beyond its limit."));
    }
    Ok(bytes)
}
pub(super) fn read_blob(root: &Path, blob: &media::Blob) -> Result<Vec<u8>> {
    let extension = if blob.path.ends_with(".wav") { "wav" } else { "json" };
    media::validate_blob(blob, extension).map_err(invalid)?;
    let bytes = read_limited(
        root,
        &blob.path,
        if extension == "wav" { media::MAX_AUDIO_BYTES } else { media::MAX_TIMING_BYTES },
    )?;
    if bytes.len() as u64 != blob.bytes || atomic::hash_bytes(&bytes) != blob.hash {
        return Err(invalid("Media bytes do not match their immutable hash and size."));
    }
    Ok(bytes)
}
pub(super) fn read_take(
    root: &Path,
    take: &media::Take,
) -> Result<(Vec<u8>, Option<media::timing::Track>)> {
    let audio = read_blob(root, &take.audio)?;
    if media::wav::inspect(&audio).map_err(invalid)? != take.info {
        return Err(invalid("Audio duration/type differs from recorded metadata."));
    }
    let timing = take
        .timing
        .as_ref()
        .map(|b| {
            read_blob(root, b).and_then(|bytes| {
                media::timing::decode(&bytes, &take.audio.hash, take.info.duration_ms)
                    .map_err(invalid)
            })
        })
        .transpose()?;
    Ok((audio, timing))
}
pub(super) struct Prepared {
    pub audio: Vec<u8>,
    pub timing: Option<Vec<u8>>,
    pub take: media::Take,
}
pub(super) fn prepare(root: &Path, row: &Row, actor: &str) -> Result<Prepared> {
    let audio = read_limited(root, &row.audio_path, media::MAX_AUDIO_BYTES)?;
    let info = media::wav::inspect(&audio).map_err(invalid)?;
    let hash = atomic::hash_bytes(&audio);
    if row.audio_hash.as_ref().is_some_and(|expected| expected != &hash) {
        return Err(invalid("Audio hash differs from the manifest."));
    }
    let timing = row
        .timing_path
        .as_ref()
        .map(|path| read_limited(root, path, media::MAX_TIMING_BYTES))
        .transpose()?;
    if row.timing_hash.is_some() && timing.is_none() {
        return Err(invalid("Timing hash requires a timing file."));
    }
    if let Some(bytes) = &timing {
        media::timing::decode(bytes, &hash, info.duration_ms).map_err(invalid)?;
        if row.timing_hash.as_ref().is_some_and(|expected| expected != &atomic::hash_bytes(bytes)) {
            return Err(invalid("Timing hash differs from the manifest."));
        }
    }
    let blob = |bytes: &[u8], ext: &str| {
        let hash = atomic::hash_bytes(bytes);
        media::Blob { path: format!("assets/media/{hash}.{ext}"), hash, bytes: bytes.len() as u64 }
    };
    let take = media::Take {
        row: row.clone(),
        spoken_text: row.spoken_text().map_err(invalid)?,
        audio: blob(&audio, "wav"),
        info,
        timing: timing.as_ref().map(|bytes| blob(bytes, "json")),
        actor: actor.into(),
    };
    Ok(Prepared { audio, timing, take })
}
pub(super) fn publish(root: &Path, blob: &media::Blob, bytes: &[u8]) -> Result<()> {
    safe_path(root, &blob.path)?;
    let relative = atomic::ProjectRelativePath::new(&blob.path)?;
    atomic::publish_content_addressed(
        root,
        &relative,
        bytes,
        |bytes| {
            if atomic::hash_bytes(bytes) != blob.hash {
                return Err(invalid("Prepared media changed before publication."));
            }
            Ok(())
        },
        |_| {
            read_blob(root, blob)?;
            Ok(atomic::ExistingContent::Valid)
        },
    )?;
    Ok(())
}

pub(super) fn check_blob_metadata(root: &Path, blob: &media::Blob) -> Result<()> {
    media::validate_blob(blob, if blob.path.ends_with(".wav") { "wav" } else { "json" })
        .map_err(invalid)?;
    let path = safe_path(root, &blob.path)?;
    let meta = std::fs::symlink_metadata(&path).map_err(|e| Error::io(&path, e))?;
    if !meta.is_file() || meta.len() != blob.bytes {
        return Err(invalid(
            "Recording file is missing or its byte size changed; audition/export verify content hashes.",
        ));
    }
    Ok(())
}
pub(super) fn check_take_metadata(root: &Path, take: &media::Take) -> Result<()> {
    for blob in std::iter::once(&take.audio).chain(take.timing.iter()) {
        check_blob_metadata(root, blob)?;
    }
    Ok(())
}
