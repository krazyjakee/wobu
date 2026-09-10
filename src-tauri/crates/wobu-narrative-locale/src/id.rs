use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// One locale, in canonical `language[-Script][-REGION]` form.
///
/// Stored as the canonical string rather than as three fields because that is
/// what every consumer needs — a map key, a file name, a CSV cell, a manifest
/// entry — and a struct that had to be re-rendered at each of those is a struct
/// whose renderings can disagree. The parts are recoverable with
/// [`LocaleId::language`] and friends, which read the string back rather than
/// storing it twice.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocaleId(String);

impl LocaleId {
    /// Parse and canonicalise a tag.
    ///
    /// Accepts `-` or `_` as the separator and any casing; produces exactly one
    /// spelling for each locale. Refuses anything outside
    /// `language[-Script][-REGION]` with a message naming what was expected,
    /// because "invalid locale" leaves a person with a spreadsheet no way to
    /// find out which cell is wrong.
    pub fn parse(value: &str) -> Result<LocaleId> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(Error::Locale { value: value.to_string() });
        }
        let parts: Vec<&str> = trimmed.split(['-', '_']).collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(Error::Locale { value: value.to_string() });
        }
        let language = parts[0];
        if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic())
        {
            return Err(Error::Locale { value: value.to_string() });
        }
        let mut canonical = language.to_ascii_lowercase();
        let mut rest = &parts[1..];

        // Script before region, and each admitted at most once. The shapes do
        // not overlap — four letters, versus two letters or three digits — so
        // this is a classification rather than a guess about intent.
        if let Some(first) = rest.first()
            && is_script(first)
        {
            canonical.push('-');
            canonical.push_str(&titlecase(first));
            rest = &rest[1..];
        }
        if let Some(first) = rest.first() {
            if !is_region(first) {
                return Err(Error::Locale { value: value.to_string() });
            }
            canonical.push('-');
            canonical.push_str(&first.to_ascii_uppercase());
            rest = &rest[1..];
        }
        if !rest.is_empty() {
            return Err(Error::Locale { value: value.to_string() });
        }
        Ok(LocaleId(canonical))
    }

    /// The canonical tag.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The language subtag on its own — `pt` for `pt-BR`.
    pub fn language(&self) -> &str {
        self.0.split('-').next().unwrap_or(&self.0)
    }

    /// The script subtag, if the tag carries one.
    pub fn script(&self) -> Option<&str> {
        self.0.split('-').find(|part| is_script(part))
    }

    /// The region subtag, if the tag carries one.
    pub fn region(&self) -> Option<&str> {
        self.0.split('-').skip(1).find(|part| is_region(part))
    }

    /// The lowercase form used as a file name stem.
    ///
    /// A separate rendering from [`LocaleId::as_str`] on purpose: canonical BCP
    /// 47 mixes case, and two tags that differ only in case cannot both exist —
    /// but a case-insensitive filesystem cannot be relied on to say so, and a
    /// project that opened on Linux and lost a pack on macOS would be the worst
    /// possible way to find out. The tag inside the document is canonical; the
    /// name on disk is unambiguous.
    pub fn file_stem(&self) -> String {
        self.0.to_ascii_lowercase()
    }

    /// This locale, then each truncation of it, ending at the bare language.
    ///
    /// The source locale is *not* appended here — [`fallback_chain`] does that —
    /// because this function is also what decides whether one locale is a
    /// generalisation of another, and a source locale in the middle of that
    /// answer would make every locale look related to every other.
    pub fn truncations(&self) -> Vec<LocaleId> {
        let parts: Vec<&str> = self.0.split('-').collect();
        (1..=parts.len()).rev().map(|take| LocaleId(parts[..take].join("-"))).collect()
    }

    /// Whether text written for `self` may stand in for `other` under the
    /// truncation rule — that is, whether `self` is `other` or a
    /// generalisation of it.
    pub fn covers(&self, other: &LocaleId) -> bool {
        other.truncations().contains(self)
    }
}

/// Whether a subtag has the shape of an ISO 15924 script code.
fn is_script(part: &str) -> bool {
    part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic())
}

/// Whether a subtag has the shape of an ISO 3166-1 alpha-2 or UN M.49 region.
fn is_region(part: &str) -> bool {
    (part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
        || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit()))
}

fn titlecase(part: &str) -> String {
    let mut out = part.to_ascii_lowercase();
    out[..1].make_ascii_uppercase();
    out
}

/// The complete lookup order for one target locale.
///
/// The rule stated in the module documentation, in one function so that no
/// caller can implement four fifths of it. The source locale is appended unless
/// it is already in the chain, and the result never contains a duplicate — a
/// chain with a repeated entry would make a fallback report say a line fell
/// back to itself.
pub fn fallback_chain(target: &LocaleId, source: &LocaleId) -> Vec<LocaleId> {
    let mut chain = target.truncations();
    if !chain.contains(source) {
        chain.push(source.clone());
    }
    chain
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for LocaleId {
    type Err = Error;

    fn from_str(value: &str) -> Result<LocaleId> {
        LocaleId::parse(value)
    }
}

impl Serialize for LocaleId {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Deserialization re-parses rather than trusting the stored string.
///
/// A pack whose locale field was hand-edited to `PT_br` would otherwise become
/// a second key for a locale that already has a pack, and the two would take
/// turns being the one the release gate reads.
impl<'de> Deserialize<'de> for LocaleId {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<LocaleId, D::Error> {
        let raw = String::deserialize(deserializer)?;
        LocaleId::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The locale a project writes its source in when it has never said otherwise.
///
/// `en`, matching the `strings/en.json` a native package has always shipped
/// (#160). A different default would mean an existing project's package and its
/// first locale pack disagreed about which column held the words the writer
/// actually typed.
pub fn default_source() -> LocaleId {
    LocaleId("en".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_has_exactly_one_canonical_spelling() {
        for spelling in ["pt-BR", "pt_br", "PT-br", " pt-BR "] {
            assert_eq!(LocaleId::parse(spelling).unwrap().as_str(), "pt-BR");
        }
    }

    #[test]
    fn a_script_is_titlecased_and_a_region_uppercased() {
        let tag = LocaleId::parse("zh-hant-hk").unwrap();
        assert_eq!(tag.as_str(), "zh-Hant-HK");
        assert_eq!(tag.language(), "zh");
        assert_eq!(tag.script(), Some("Hant"));
        assert_eq!(tag.region(), Some("HK"));
    }

    #[test]
    fn a_numeric_region_is_admitted() {
        assert_eq!(LocaleId::parse("es-419").unwrap().as_str(), "es-419");
    }

    #[test]
    fn anything_outside_the_admitted_shape_is_refused() {
        for bad in ["", "e", "english-language", "en-US-POSIX", "x-private", "en--US", "1n"] {
            assert!(LocaleId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_chain_truncates_and_then_reaches_the_source() {
        let chain = fallback_chain(
            &LocaleId::parse("zh-Hant-HK").unwrap(),
            &LocaleId::parse("en").unwrap(),
        );
        let rendered: Vec<&str> = chain.iter().map(LocaleId::as_str).collect();
        assert_eq!(rendered, ["zh-Hant-HK", "zh-Hant", "zh", "en"]);
    }

    #[test]
    fn the_chain_never_moves_sideways() {
        // The whole point of the rule: pt-BR must not reach pt-PT, however
        // tempting the near-match is.
        let chain =
            fallback_chain(&LocaleId::parse("pt-BR").unwrap(), &LocaleId::parse("en").unwrap());
        assert!(!chain.contains(&LocaleId::parse("pt-PT").unwrap()));
    }

    #[test]
    fn a_target_that_generalises_to_the_source_does_not_repeat_it() {
        let chain =
            fallback_chain(&LocaleId::parse("en-GB").unwrap(), &LocaleId::parse("en").unwrap());
        let rendered: Vec<&str> = chain.iter().map(LocaleId::as_str).collect();
        assert_eq!(rendered, ["en-GB", "en"]);
    }

    #[test]
    fn coverage_runs_from_general_to_specific_only() {
        let general = LocaleId::parse("pt").unwrap();
        let specific = LocaleId::parse("pt-BR").unwrap();
        assert!(general.covers(&specific));
        assert!(!specific.covers(&general));
    }

    #[test]
    fn a_file_stem_is_unambiguous_on_a_case_insensitive_filesystem() {
        assert_eq!(LocaleId::parse("pt-BR").unwrap().file_stem(), "pt-br");
    }

    #[test]
    fn a_stored_tag_is_re_parsed_rather_than_trusted() {
        let parsed: LocaleId = serde_json::from_str("\"PT_br\"").unwrap();
        assert_eq!(parsed.as_str(), "pt-BR");
        assert!(serde_json::from_str::<LocaleId>("\"not a locale\"").is_err());
    }
}
