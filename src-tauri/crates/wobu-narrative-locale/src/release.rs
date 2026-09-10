//! Prepared strings only. The host selects a plural category; missing categories use other.
use crate::{Diagnostic, LocaleId, PluralCategory, Policy, SourceLine, Translation, VERSION};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    pub locale: LocaleId,
    pub forms: BTreeMap<PluralCategory, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub version: u32,
    pub policy: Policy,
    pub strings: BTreeMap<LocaleId, BTreeMap<String, Localized>>,
}
pub fn prepare(
    policy: &Policy,
    sources: &BTreeMap<String, SourceLine>,
    translations: &BTreeMap<(LocaleId, String), Translation>,
) -> (Bundle, Vec<Diagnostic>) {
    let mut bundle = Bundle { version: VERSION, policy: policy.clone(), strings: BTreeMap::new() };
    let mut diagnostics = Vec::new();
    for (locale, allow_fallback) in &policy.required {
        let mut strings = BTreeMap::new();
        for source in sources.values() {
            let chain = if *allow_fallback {
                crate::id::fallback_chain(locale, &policy.source)
            } else {
                vec![locale.clone()]
            };
            let found = chain.into_iter().find_map(|candidate| {
                if candidate == policy.source {
                    return Some(Localized {
                        locale: candidate,
                        forms: BTreeMap::from([(PluralCategory::Other, source.text.clone())]),
                    });
                }
                translations
                    .get(&(candidate.clone(), source.id.clone()))
                    .filter(|t| t.current(source) && t.latest().approved)
                    .map(|t| Localized { locale: candidate, forms: t.latest().forms.clone() })
            });
            match found {
                Some(value) => {
                    if &value.locale != locale {
                        diagnostics.push(Diagnostic {
                            id: source.id.clone(),
                            code: "fallback".into(),
                            message: format!(
                                "{locale}: using explicitly permitted {} fallback.",
                                value.locale
                            ),
                        });
                    }
                    strings.insert(source.id.clone(), value);
                }
                None => diagnostics.push(Diagnostic {
                    id: source.id.clone(),
                    code: "missing_translation".into(),
                    message: format!(
                        "{locale}: current approved translation required; fallback is disabled."
                    ),
                }),
            }
        }
        bundle.strings.insert(locale.clone(), strings);
    }
    (bundle, diagnostics)
}
impl Bundle {
    pub fn text(&self, locale: &LocaleId, id: &str, category: PluralCategory) -> Option<&str> {
        let forms = &self.strings.get(locale)?.get(id)?.forms;
        forms.get(&category).or_else(|| forms.get(&PluralCategory::Other)).map(String::as_str)
    }
    /// Exact named-token substitution. Values are already formatted by the host; never HTML.
    pub fn render(
        &self,
        locale: &LocaleId,
        id: &str,
        category: PluralCategory,
        values: &BTreeMap<String, String>,
    ) -> crate::Result<String> {
        let text = self.text(locale, id, category).ok_or_else(|| {
            crate::Error::Malformed("Unknown packaged locale or string ID.".into())
        })?;
        let tokens = crate::format::placeholders(text);
        if tokens.iter().any(|token| !values.contains_key(token)) {
            return Err(crate::Error::Malformed("Missing named formatting value.".into()));
        }
        let mut out = String::new();
        let mut rest = text;
        while !rest.is_empty() {
            if rest.starts_with("{{") || rest.starts_with("}}") {
                out.push(rest.chars().next().unwrap());
                rest = &rest[2..];
                continue;
            }
            if rest.starts_with('{')
                && let Some(end) = rest.find('}')
                && tokens.contains(&rest[1..end])
                && let Some(value) = values.get(&rest[1..end])
            {
                out.push_str(value);
                rest = &rest[end + 1..];
                continue;
            }
            let c = rest.chars().next().unwrap();
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
        Ok(out)
    }
}
