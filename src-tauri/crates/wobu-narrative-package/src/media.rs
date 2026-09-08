use crate::*;
use wobu_narrative_media::{self as media, release::Bundle};
pub const PREPARED_MEDIA: &str = "prepared_media";
impl Package {
    pub fn with_media(mut self, bundle: Bundle, files: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        if bundle.required.is_empty() && bundle.takes.is_empty() && bundle.fallback.is_empty() {
            return Ok(self);
        }
        for (path, bytes) in files {
            if !path.starts_with("assets/media/") || self.files.insert(path, bytes).is_some() {
                return Err(invalid("Duplicate or invalid prepared media path."));
            }
        }
        self.files.insert("media.json".into(), json(&bundle)?);
        self.manifest.required_capabilities.insert(PREPARED_MEDIA.into(), 1);
        self.rehash();
        self.graph()?;
        Ok(self)
    }
    pub fn media(&self) -> Result<Option<Bundle>> {
        if !self.manifest.required_capabilities.contains_key(PREPARED_MEDIA) {
            let empty: Media = parse(self.file("media.json")?)?;
            if !empty.is_empty() {
                return Err(invalid("Media bindings require the prepared_media capability."));
            }
            return Ok(None);
        }
        let bundle: Bundle = parse(self.file("media.json")?)?;
        if bundle.version != 1
            || bundle.timing.iter().any(|locale| !bundle.required.contains_key(locale))
        {
            return Err(invalid("Unsupported prepared media version."));
        }
        let source = self.strings()?;
        let locales = self.locales()?;
        let mut referenced = std::collections::BTreeSet::new();
        for (key, take) in &bundle.takes {
            if key != &take.key.token() || take.origin != take.key.locale {
                return Err(invalid("Media identity or locale origin mismatch."));
            }
            let original =
                source.get(&take.key.id).ok_or_else(|| invalid("Unknown media string ID."))?;
            if original.revision.as_ref().is_some_and(|r| r != &take.source_revision) {
                return Err(invalid("Media source revision mismatch."));
            }
            let template = if take.key.locale.as_str() == self.manifest.locale {
                original.text.as_str()
            } else {
                locales
                    .as_ref()
                    .and_then(|b| b.strings.get(&take.key.locale))
                    .and_then(|rows| rows.get(&take.key.id))
                    .filter(|value| value.locale == take.origin)
                    .and_then(|v| v.forms.get(&take.key.form))
                    .map(String::as_str)
                    .ok_or_else(|| {
                        invalid("Media locale/form is not in the prepared string tables.")
                    })?
            };
            if template != take.template {
                return Err(invalid("Media wording differs from packaged strings."));
            }
            let tokens = wobu_narrative_locale::format::placeholders(template);
            if tokens != take.parameters.keys().cloned().collect() {
                return Err(invalid("Media parameters do not freeze the packaged template."));
            }
            let rendered = wobu_narrative_locale::release::Bundle {
                version: 1,
                policy: wobu_narrative_locale::Policy::default(),
                strings: BTreeMap::from([(
                    take.key.locale.clone(),
                    BTreeMap::from([(
                        take.key.id.clone(),
                        wobu_narrative_locale::release::Localized {
                            locale: take.origin.clone(),
                            forms: BTreeMap::from([(take.key.form, template.to_string())]),
                        },
                    )]),
                )]),
            }
            .render(&take.key.locale, &take.key.id, take.key.form, &take.parameters)
            .map_err(|e| invalid(e.to_string()))?;
            if rendered != take.spoken_text {
                return Err(invalid("Prepared spoken text disagrees with frozen parameters."));
            }
            media::validate_blob(&take.audio, "wav").map_err(|e| invalid(e.to_string()))?;
            let audio = self.file(&take.audio.path)?;
            if hash(audio) != take.audio.hash
                || audio.len() as u64 != take.audio.bytes
                || media::wav::inspect(audio).map_err(|e| invalid(e.to_string()))? != take.info
            {
                return Err(invalid("Prepared audio hash, size or duration mismatch."));
            }
            referenced.insert(take.audio.path.clone());
            if let Some(timing) = &take.timing {
                media::validate_blob(timing, "json").map_err(|e| invalid(e.to_string()))?;
                let bytes = self.file(&timing.path)?;
                if hash(bytes) != timing.hash || bytes.len() as u64 != timing.bytes {
                    return Err(invalid("Timing hash or size mismatch."));
                }
                media::timing::decode(bytes, &take.audio.hash, take.info.duration_ms)
                    .map_err(|e| invalid(e.to_string()))?;
                referenced.insert(timing.path.clone());
            }
        }
        if self
            .files
            .keys()
            .filter(|p| p.starts_with("assets/media/"))
            .any(|p| !referenced.contains(p))
        {
            return Err(invalid("Unreferenced packaged media file."));
        }
        for key in &bundle.fallback {
            let Some((locale, _)) = key.split_once('/') else {
                return Err(invalid("Invalid media fallback key."));
            };
            let locale =
                locale.parse().map_err(|e: wobu_narrative_locale::Error| invalid(e.to_string()))?;
            if bundle.required.get(&locale) != Some(&true)
                || bundle.takes.get(key).is_some_and(|take| {
                    wobu_narrative_locale::format::placeholders(&take.template).is_empty()
                })
            {
                return Err(invalid("Media fallback is not explicitly permitted."));
            }
        }
        if self.manifest.profile == Profile::Release {
            for (locale, allow_fallback) in &bundle.required {
                for (id, text) in &source {
                    let forms = if locale.as_str() == self.manifest.locale {
                        vec![wobu_narrative_locale::PluralCategory::Other]
                    } else {
                        locales
                            .as_ref()
                            .and_then(|b| b.strings.get(locale))
                            .and_then(|rows| rows.get(id))
                            .map(|v| v.forms.keys().copied().collect())
                            .ok_or_else(|| invalid("Required recording locale is absent."))?
                    };
                    let _ = text;
                    for form in forms {
                        let key =
                            media::Key { id: id.clone(), locale: locale.clone(), form }.token();
                        if let Some(take) = bundle.takes.get(&key) {
                            if !*allow_fallback
                                && !wobu_narrative_locale::format::placeholders(&take.template)
                                    .is_empty()
                            {
                                return Err(invalid(
                                    "Dynamic recordings require explicit text-only fallback.",
                                ));
                            }
                            if bundle.timing.contains(locale) && take.timing.is_none() {
                                return Err(invalid("Required timing is missing."));
                            }
                        }
                        if !bundle.takes.contains_key(&key)
                            && !(*allow_fallback && bundle.fallback.contains(&key))
                        {
                            return Err(invalid(
                                "Release requires current media or explicit text-only fallback.",
                            ));
                        }
                    }
                }
            }
        }
        Ok(Some(bundle))
    }
    pub fn media_audio(&self, take: &media::release::Prepared) -> Result<&[u8]> {
        self.file(&take.audio.path)
    }
    pub fn media_timing(
        &self,
        take: &media::release::Prepared,
    ) -> Result<Option<media::timing::Track>> {
        take.timing
            .as_ref()
            .map(|blob| {
                media::timing::decode(
                    self.file(&blob.path)?,
                    &take.audio.hash,
                    take.info.duration_ms,
                )
                .map_err(|e| invalid(e.to_string()))
            })
            .transpose()
    }
}
