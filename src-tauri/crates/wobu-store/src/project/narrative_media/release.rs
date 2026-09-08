use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug)]
pub struct MediaRelease {
    pub bundle: media::release::Bundle,
    pub files: BTreeMap<String, Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
}
impl Project {
    pub fn media_release(&self) -> Result<MediaRelease> {
        let (policy, guard) = self.media_policy()?;
        let bindings = self.media_bindings()?;
        let mut result = MediaRelease {
            bundle: media::release::Bundle {
                version: 1,
                required: policy.required.clone(),
                timing: policy.timing.clone(),
                takes: BTreeMap::new(),
                fallback: BTreeSet::new(),
            },
            files: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        if policy.required.is_empty() && bindings.is_empty() {
            return Ok(result);
        }
        let capture = self.production_capture()?;
        let locales: BTreeSet<_> = policy
            .required
            .keys()
            .cloned()
            .chain(bindings.values().map(|b| b.key.locale.clone()))
            .collect();
        for locale in locales {
            if locale != capture.policy.source && !capture.policy.required.contains_key(&locale) {
                if policy.required.contains_key(&locale) {
                    result.diagnostics.push(Diagnostic::new(
                        locale.to_string(),
                        "missing_media",
                        "Configure this recording locale in the locale Release policy first.",
                    ));
                }
                continue;
            }
            for row in media::rows(
                &locale,
                &capture.policy.source,
                &policy,
                &capture.sources,
                &capture.translations,
                &bindings,
            ) {
                let key = row.key.token();
                let dynamic = !wobu_narrative_locale::format::placeholders(&row.text).is_empty();
                if dynamic && policy.required.get(&locale) == Some(&false) {
                    result.diagnostics.push(Diagnostic::new(&key,"missing_media","Dynamic templates require explicit text-only fallback; a frozen take covers only its exact parameters."));
                    continue;
                }
                if dynamic && policy.required.get(&locale) == Some(&true) {
                    result.bundle.fallback.insert(key.clone());
                }
                let candidate = bindings
                    .get(&key)
                    .filter(|binding| binding.current(&row))
                    .and_then(Binding::latest)
                    .filter(|take| !policy.timing.contains(&locale) || take.timing.is_some());
                if let Some(take) = candidate {
                    let retained: u64 = result.files.values().map(|bytes| bytes.len() as u64).sum();
                    let additions = std::iter::once(&take.audio)
                        .chain(take.timing.iter())
                        .filter(|blob| !result.files.contains_key(&blob.path))
                        .map(|blob| blob.bytes);
                    check_budget(retained, additions)?;
                }
                let available = candidate
                    .map(|take| files::read_take(self.root(), take).map(|(audio, _)| (take, audio)))
                    .transpose();
                match available {
                    Ok(Some((take, audio))) => {
                        result.files.insert(take.audio.path.clone(), audio);
                        if let Some(blob) = &take.timing {
                            result
                                .files
                                .insert(blob.path.clone(), files::read_blob(self.root(), blob)?);
                        }
                        result.bundle.takes.insert(key, media::release::Prepared::from(take));
                    }
                    other => {
                        let reason = match other {
                            Err(e) => e.to_string(),
                            _ => {
                                "Missing or outdated recording for exact source/translation.".into()
                            }
                        };
                        if let Some(fallback) = policy.required.get(&locale) {
                            let code = if *fallback {
                                result.bundle.fallback.insert(key.clone());
                                "text_fallback"
                            } else {
                                "missing_media"
                            };
                            result.diagnostics.push(Diagnostic::new(key, code, reason));
                        }
                    }
                }
            }
        }
        // Missing translations must not make a required recording disappear from release checks.
        capture.verify(self)?;
        if self.media_policy()?.1 != guard || self.media_bindings()? != bindings {
            return Err(invalid("Recording policy/history changed during export capture."));
        }
        // Verify immutable files after assembling the complete package input.
        for (path, bytes) in &result.files {
            if files::read_limited(self.root(), path, media::MAX_AUDIO_BYTES)? != *bytes {
                return Err(invalid("Media blob changed during export capture."));
            }
        }
        Ok(result)
    }
}

fn check_budget(retained: u64, mut additions: impl Iterator<Item = u64>) -> Result<()> {
    let total = additions.try_fold(retained, |sum, bytes| sum.checked_add(bytes));
    if total.is_none_or(|total| total > 120 * 1024 * 1024) {
        return Err(invalid("Prepared media exceeds the 120 MiB aggregate export budget."));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_refuses_before_reading_or_allocating_the_next_blob() {
        assert!(check_budget(100 * 1024 * 1024, [20 * 1024 * 1024].into_iter()).is_ok());
        assert!(check_budget(100 * 1024 * 1024, [21 * 1024 * 1024].into_iter()).is_err());
        assert!(check_budget(u64::MAX, [1].into_iter()).is_err());
    }
}
