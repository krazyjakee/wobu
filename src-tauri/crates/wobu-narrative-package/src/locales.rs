use crate::*;
use wobu_narrative_locale::{LocaleId, PluralCategory, release::Bundle};
pub const LOCALISATION: &str = "localisation";
impl Package {
    /// Attach resolved, release-gated strings. Canonical source tables remain available.
    pub fn with_locales(mut self, bundle: Bundle) -> Result<Self> {
        bundle.policy.validate().map_err(|e| invalid(e.to_string()))?;
        if bundle.policy.required.is_empty() && bundle.policy.source.as_str() == "en" {
            return Ok(self);
        }
        let old_source = format!("strings/{}.json", self.manifest.locale.to_ascii_lowercase());
        let source_bytes =
            self.files.remove(&old_source).ok_or_else(|| invalid("Source strings missing."))?;
        self.files
            .insert(format!("strings/{}.json", bundle.policy.source.file_stem()), source_bytes);
        self.manifest.locale = bundle.policy.source.to_string();
        self.files.insert("locales.json".into(), json(&bundle)?);
        self.manifest.required_capabilities.insert(LOCALISATION.into(), 1);
        self.rehash();
        self.graph()?;
        Ok(self)
    }
    fn rehash(&mut self) {
        self.manifest.files = self
            .files
            .iter()
            .map(|(path, bytes)| {
                (path.clone(), FileRecord { bytes: bytes.len() as u64, hash: hash(bytes) })
            })
            .collect();
        self.manifest.payload_hash =
            hash(&json(&self.manifest.files).expect("file records serialize"));
    }
    pub fn locales(&self) -> Result<Option<Bundle>> {
        if !self.files.contains_key("locales.json") {
            return Ok(None);
        }
        let bundle: Bundle = parse(self.file("locales.json")?)?;
        bundle.policy.validate().map_err(|e| invalid(e.to_string()))?;
        if bundle.policy.source.as_str() != self.manifest.locale {
            return Err(invalid("Source locale differs from policy."));
        }
        if bundle.version != wobu_narrative_locale::VERSION
            || bundle.strings.keys().ne(bundle.policy.required.keys())
        {
            return Err(invalid("Locale bundle version or configured locales disagree."));
        }
        let source = self.strings()?;
        for (locale, rows) in &bundle.strings {
            if rows.keys().ne(source.keys()) {
                return Err(invalid("Locale bundle IDs do not exactly match source strings."));
            }
            for (id, value) in rows {
                if !value.forms.contains_key(&PluralCategory::Other)
                    || value.forms.values().any(|text| {
                        text.trim().is_empty()
                            || text.len() > MAX_STRING_BYTES
                            || !wobu_narrative_locale::format::compare(&source[id].text, text)
                                .is_empty()
                    })
                {
                    return Err(invalid(
                        "Locale form is empty, oversized or changes placeholders.",
                    ));
                }
                let allowed = if bundle.policy.required[locale] {
                    wobu_narrative_locale::id::fallback_chain(locale, &bundle.policy.source)
                } else {
                    vec![locale.clone()]
                };
                if !allowed.contains(&value.locale) {
                    return Err(invalid("Locale uses fallback forbidden by policy."));
                }
            }
        }
        Ok(Some(bundle))
    }
    /// Reference native lookup: ordinary graph execution uses the `other` form.
    /// Hosts can select another category with Bundle::render before displaying a yielded line.
    pub fn graph_locale(&self, locale: &LocaleId) -> Result<Graph> {
        let mut graph = self.graph()?;
        let bundle =
            self.locales()?.ok_or_else(|| invalid("Package has no configured locales."))?;
        if locale == &bundle.policy.source {
            return Ok(graph);
        }
        let text = |id: &str| {
            bundle
                .text(locale, id, PluralCategory::Other)
                .map(str::to_string)
                .ok_or_else(|| invalid("Requested locale or string is not in this package."))
        };
        for scene in graph.scenes.values_mut() {
            for beat in scene.beats.values_mut() {
                for slot in &mut beat.dialogue {
                    for variant in &mut slot.variants {
                        variant.text = text(&variant.id)?;
                    }
                }
                for choice in &mut beat.choices {
                    choice.label = text(&choice.id)?;
                }
            }
        }
        for asset in graph.texts.values_mut() {
            for entry in &mut asset.entries {
                for slot in &mut entry.lines {
                    for variant in &mut slot.variants {
                        variant.text = text(&variant.id)?;
                    }
                }
            }
        }
        validate::graph(&graph)?;
        Ok(graph)
    }
}
