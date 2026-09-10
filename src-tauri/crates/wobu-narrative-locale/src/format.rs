use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The CLDR plural categories, and no others.
///
/// A closed enum rather than a free string, because the set is the contract:
/// a pack carrying a category no host recognises is a form that will never be
/// selected, which is a translation nobody will ever see and nobody will ever
/// be told about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    /// The base form. Always present, and the fallback for every other
    /// category — see the module documentation.
    Other,
}

impl PluralCategory {
    /// Every category, in CLDR's own order.
    pub const ALL: [PluralCategory; 6] = [
        PluralCategory::Zero,
        PluralCategory::One,
        PluralCategory::Two,
        PluralCategory::Few,
        PluralCategory::Many,
        PluralCategory::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            PluralCategory::Zero => "zero",
            PluralCategory::One => "one",
            PluralCategory::Two => "two",
            PluralCategory::Few => "few",
            PluralCategory::Many => "many",
            PluralCategory::Other => "other",
        }
    }
}

impl fmt::Display for PluralCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PluralCategory {
    type Err = Error;

    fn from_str(value: &str) -> Result<PluralCategory> {
        PluralCategory::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
            .ok_or_else(|| Error::PluralCategory(value.to_string()))
    }
}

/// Every placeholder token in a piece of text, as a set.
///
/// A set and not a list, because the rule this exists to serve is set equality.
/// Returning a list would invite a caller to compare sequences, which would
/// refuse every translation that reordered a sentence — see the module
/// documentation for why that is the single most important thing not to do
/// here.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != '{' {
            index += 1;
            continue;
        }
        // `{{` is a literal brace and cannot open a placeholder. Skipping both
        // characters is what stops `{{name}}` being read as a substitution — a
        // reading that would make every escaped brace a mismatch.
        if bytes.get(index + 1) == Some(&'{') {
            index += 2;
            continue;
        }
        let Some(close) = (index + 1..bytes.len()).find(|&at| bytes[at] == '}') else {
            index += 1;
            continue;
        };
        let inner: String = bytes[index + 1..close].iter().collect();
        if inner.contains('{') || inner.contains('\n') || inner.contains('\r') {
            index += 1;
            continue;
        }
        if is_token(&inner) {
            found.insert(inner);
            index = close + 1;
        } else {
            index += 1;
        }
    }
    found
}

/// Whether the inside of a brace pair is a well-formed `name` or `name:format`.
fn is_token(inner: &str) -> bool {
    let (name, format) = match inner.split_once(':') {
        Some((name, format)) => (name, Some(format)),
        None => (inner, None),
    };
    let mut chars = name.chars();
    let starts = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let continues = chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    starts && continues && format.is_none_or(|hint| !hint.is_empty())
}

/// How a translation's placeholders differ from its source's.
///
/// Both directions are reported rather than a single "mismatch" flag, because
/// they are different mistakes with different fixes: a missing placeholder is
/// usually a translator deleting something that looked like markup, and an
/// unexpected one is usually a typo in a name. Telling somebody which is which
/// is the difference between a diagnostic they can act on and one they have to
/// investigate.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceholderDelta {
    /// In the source, absent from the translation.
    pub missing: Vec<String>,
    /// In the translation, absent from the source.
    pub unexpected: Vec<String>,
}

impl PlaceholderDelta {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unexpected.is_empty()
    }
}

/// Apply the placeholder rule to one pair of strings.
pub fn compare(source: &str, translation: &str) -> PlaceholderDelta {
    let expected = placeholders(source);
    let actual = placeholders(translation);
    PlaceholderDelta {
        missing: expected.difference(&actual).cloned().collect(),
        unexpected: actual.difference(&expected).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn a_named_placeholder_is_found() {
        assert_eq!(
            placeholders("Hello {name}, you owe {amount:money}."),
            set(&["amount:money", "name"])
        );
    }

    #[test]
    fn an_escaped_brace_is_not_a_placeholder() {
        assert!(placeholders("Use {{name}} to insert a name.").is_empty());
    }

    #[test]
    fn prose_that_merely_contains_braces_is_left_alone() {
        // A writer describing a syntax, or a stray keystroke. Neither is a
        // substitution, and neither may become one.
        assert!(placeholders("The sigil { was carved deep.").is_empty());
        assert!(placeholders("A set is written {1, 2, 3}.").is_empty());
        assert!(placeholders("{ spaced }").is_empty());
    }

    #[test]
    fn reordering_and_repeating_are_allowed() {
        let delta = compare("{a} then {b}", "{b}, {b} — and only then {a}");
        assert!(delta.is_empty(), "{delta:?}");
    }

    #[test]
    fn a_dropped_placeholder_is_reported_as_missing() {
        let delta = compare("You found {count} of {total}.", "Encontraste {count}.");
        assert_eq!(delta.missing, ["total"]);
        assert!(delta.unexpected.is_empty());
    }

    #[test]
    fn an_invented_placeholder_is_reported_as_unexpected() {
        let delta = compare("You found {count}.", "Encontraste {contagem}.");
        assert_eq!(delta.missing, ["count"]);
        assert_eq!(delta.unexpected, ["contagem"]);
    }

    #[test]
    fn a_dropped_format_hint_is_a_mismatch() {
        let delta = compare("{count:number} ships", "{count} navios");
        assert_eq!(delta.missing, ["count:number"]);
        assert_eq!(delta.unexpected, ["count"]);
    }

    #[test]
    fn placeholders_survive_unicode_and_rtl_text() {
        // The scanner walks characters rather than bytes, so a placeholder that
        // follows a multi-byte character is still found at the right boundary.
        let delta = compare("You owe {amount} coins.", "أنت مدين بمبلغ {amount} من العملات.");
        assert!(delta.is_empty(), "{delta:?}");
    }

    #[test]
    fn a_plural_category_round_trips_through_its_name() {
        for category in PluralCategory::ALL {
            assert_eq!(category.as_str().parse::<PluralCategory>().unwrap(), category);
        }
        assert!("plural".parse::<PluralCategory>().is_err());
    }
}
