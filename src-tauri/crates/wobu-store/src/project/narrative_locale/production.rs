//! Shared production capture: one review batch and frozen source/locale observations.
use super::super::narrative_review::ReviewSnapshot;
use super::*;
pub(crate) struct ProductionCapture {
    pub sources: BTreeMap<String, SourceLine>,
    pub policy: Policy,
    pub translations: BTreeMap<(LocaleId, String), Translation>,
    snapshots: Vec<ReviewSnapshot>,
    fingerprint: String,
    policy_guard: String,
}
impl Project {
    pub(crate) fn production_policy<T: serde::de::DeserializeOwned + Default>(
        &self,
        id: Id,
    ) -> Result<T> {
        self.narrative_record(Kind::Policy, id)?
            .map(|file| {
                serde_json::from_value(file.document.payload["policy"].clone()).map_err(Error::from)
            })
            .unwrap_or_else(|| Ok(T::default()))
    }
    pub(crate) fn production_capture(&self) -> Result<ProductionCapture> {
        self.production_capture_selected(None)
    }
    pub(crate) fn production_capture_selected(
        &self,
        container: Option<wobu_narrative::SceneId>,
    ) -> Result<ProductionCapture> {
        let fingerprint = self.narrative_fingerprint()?;
        let (policy, policy_guard) = self.locale_policy()?;
        let (sources, snapshots) = match container {
            Some(id) => self.locale_capture_selected(&[id])?,
            None => self.locale_capture()?,
        };
        let translations = self.locale_translations()?;
        let capture = ProductionCapture {
            sources,
            policy,
            translations,
            snapshots,
            fingerprint,
            policy_guard,
        };
        capture.verify(self)?;
        Ok(capture)
    }
}
impl ProductionCapture {
    pub fn verify(&self, project: &Project) -> Result<()> {
        for snapshot in &self.snapshots {
            snapshot.check_observations(project)?;
        }
        if project.narrative_fingerprint()? != self.fingerprint
            || project.locale_policy()?.1 != self.policy_guard
            || project.locale_translations()? != self.translations
        {
            return Err(invalid(
                "Production source, locale policy or translations changed during capture.",
            ));
        }
        Ok(())
    }
    pub fn verify_source(
        &self,
        project: &Project,
        container: wobu_narrative::SceneId,
        id: &str,
    ) -> Result<()> {
        self.snapshots
            .iter()
            .find(|s| s.scene().id == container)
            .ok_or_else(|| invalid("Missing production source capture."))?
            .check_observations(project)?;
        if project.locale_policy()?.1 != self.policy_guard {
            return Err(invalid("Source locale policy changed during production publication."));
        }
        for ((locale, source), translation) in
            self.translations.iter().filter(|((_, source), _)| source == id)
        {
            let file = project
                .narrative_record(Kind::Production, translation_id(locale, source))?
                .ok_or_else(|| invalid("Translation disappeared during production publication."))?;
            let stored: StoredTranslation = serde_json::from_value(file.document.payload)?;
            if &stored.translation != translation {
                return Err(invalid("Translation changed during production publication."));
            }
            let receipt = project
                .narrative_record(Kind::Receipt, stored.receipt)?
                .ok_or_else(|| invalid("Translation receipt disappeared."))?;
            if receipt.document.payload
                != serde_json::json!({"type":"narrative_locale_decision","translation":translation})
            {
                return Err(invalid("Translation evidence changed during production publication."));
            }
        }
        Ok(())
    }
}
