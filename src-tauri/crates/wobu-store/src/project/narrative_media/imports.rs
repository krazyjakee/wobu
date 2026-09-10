use super::*;
use std::{collections::BTreeSet, path::Path};
impl Project {
    pub fn media_preview(
        &self,
        input: &str,
        csv: bool,
        directory: &Path,
    ) -> Result<Vec<Diagnostic>> {
        let rows = media::interchange::decode(input, csv).map_err(invalid)?;
        let capture = self.production_capture()?;
        let (policy, _) = self.media_policy()?;
        let bindings = self.media_bindings()?;
        let current = current_rows(&rows, &capture, &policy, &bindings);
        let mut diagnostics = media::preview(&rows, &current);
        let blocked: BTreeSet<_> =
            diagnostics.iter().filter(|d| d.code != "missing").map(|d| d.key.clone()).collect();
        for row in &rows {
            if !blocked.contains(&row.key.token())
                && let Err(e) = files::prepare(directory, row, &self.peer)
            {
                diagnostics.push(Diagnostic::new(row.key.token(), "invalid_media", e.to_string()));
            }
        }
        capture.verify(self)?;
        Ok(diagnostics)
    }
    pub fn media_import(
        &mut self,
        input: &str,
        csv: bool,
        directory: &Path,
    ) -> Result<ImportReport> {
        self.ensure_writable()?;
        let rows = media::interchange::decode(input, csv).map_err(invalid)?;
        let capture = self.production_capture()?;
        let (policy, policy_guard) = self.media_policy()?;
        let bindings = self.media_bindings()?;
        let current = current_rows(&rows, &capture, &policy, &bindings);
        let mut report = ImportReport {
            applied: Vec::new(),
            diagnostics: media::preview(&rows, &current),
            conflicts: BTreeMap::new(),
        };
        let blocked: BTreeSet<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code != "missing")
            .map(|d| d.key.clone())
            .collect();
        for row in &rows {
            let key = row.key.token();
            if blocked.contains(&key) {
                continue;
            }
            let result = (|| {
                let prepared = files::prepare(directory, row, &self.peer)?;
                let source = capture
                    .sources
                    .get(&row.key.id)
                    .ok_or_else(|| invalid("Unknown media source."))?;
                let container = source.container.parse().map_err(invalid)?;
                let _lock = super::super::narrative_review::scene_lock(self, container)?;
                capture.verify_source(self, container, &source.id)?;
                if self.media_policy()?.1 != policy_guard {
                    return Err(invalid("Recording policy changed during import."));
                }
                let id = binding_id(&key);
                let old = self.narrative_record(Kind::Production, id)?;
                let binding = bindings.get(&key);
                if old.as_ref().map(|f| f.document.payload["binding"].clone())
                    != binding.map(|b| serde_json::to_value(b).unwrap())
                {
                    return Err(invalid("Recording changed during import."));
                }
                files::publish(self.root(), &prepared.take.audio, &prepared.audio)?;
                if let (Some(blob), Some(bytes)) = (&prepared.take.timing, &prepared.timing) {
                    files::publish(self.root(), blob, bytes)?;
                }
                // Blob publication can be slow; protection and locale observations are checked again.
                capture.verify_source(self, container, &source.id)?;
                if self.media_policy()?.1 != policy_guard {
                    return Err(invalid("Recording policy changed before publication."));
                }
                let mut next = binding.cloned().unwrap_or_else(|| Binding {
                    version: 1,
                    key: row.key.clone(),
                    history: Vec::new(),
                });
                next.history.push(prepared.take);
                next.validate().map_err(invalid)?;
                let receipt_id = wobu_core::new_id();
                let mut receipt = File {
                    document: Document::new(
                        Kind::Receipt,
                        receipt_id,
                        "Recording decision",
                        serde_json::json!({"type":"narrative_media_decision","binding":next}),
                    ),
                    stamp: None,
                };
                if !matches!(self.save_narrative_record(&mut receipt)?, SourceSave::Saved(_)) {
                    return Err(invalid("Recording receipt could not be published."));
                }
                let mut file = File {
                    document: Document::new(
                        Kind::Production,
                        id,
                        &key,
                        serde_json::to_value(StoredBinding {
                            kind: "narrative_media".into(),
                            variant_id: row.key.id.clone(),
                            binding: next,
                            receipt: receipt_id,
                        })?,
                    ),
                    stamp: old.and_then(|f| f.stamp),
                };
                self.save_narrative_record(&mut file)
            })();
            match result {
                Ok(SourceSave::Saved(_)) => report.applied.push(key),
                Ok(SourceSave::Conflict { conflict_path }) => {
                    report.conflicts.insert(key, conflict_path);
                }
                Err(e) => report.diagnostics.push(Diagnostic::new(
                    key,
                    "conflict_or_invalid_media",
                    e.to_string(),
                )),
            }
        }
        Ok(report)
    }
}
fn current_rows(
    rows: &[Row],
    capture: &super::super::narrative_locale::ProductionCapture,
    policy: &Policy,
    bindings: &BTreeMap<String, Binding>,
) -> BTreeMap<String, Row> {
    let locales: BTreeSet<_> = rows.iter().map(|r| r.key.locale.clone()).collect();
    locales
        .iter()
        .flat_map(|locale| {
            media::rows(
                locale,
                &capture.policy.source,
                policy,
                &capture.sources,
                &capture.translations,
                bindings,
            )
        })
        .map(|row| (row.key.token(), row))
        .collect()
}
