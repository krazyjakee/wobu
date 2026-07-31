//! Filename slugs.
//!
//! Project folders are meant to sit on SMB shares and be opened from Windows,
//! so every path segment we write has to survive the most restrictive
//! filesystem in the chain, not the one we happen to be running on. The rules
//! come from `docs/02-data-model.md`.

use crate::error::{Error, Result};

/// Names Windows refuses to create, with or without an extension.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Kept well under Windows' 260-character total path limit, which the whole
/// project path plus `nodes/<kind>/<slug>.md` has to fit inside.
const MAX_LEN: usize = 64;

/// Reduce a display name to a lowercase ASCII slug safe on every target
/// filesystem.
///
/// Returns [`Error::UnslugifiableName`] when nothing usable survives — a name
/// of entirely non-ASCII characters, for example. Callers should fall back to
/// the node's ULID in that case rather than guessing.
pub fn slugify(name: &str) -> Result<String> {
    let mut out = String::with_capacity(name.len());
    let mut pending_sep = false;

    for ch in name.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(ch);
        } else {
            // Everything else — spaces, punctuation, the Windows-forbidden set,
            // and any non-ASCII character — collapses into a single separator.
            pending_sep = true;
        }
    }

    if out.len() > MAX_LEN {
        out.truncate(MAX_LEN);
        // Truncation can leave a dangling separator.
        while out.ends_with('-') {
            out.pop();
        }
    }

    if out.is_empty() {
        return Err(Error::UnslugifiableName(name.to_string()));
    }

    // A reserved name is only reserved as a whole segment, so a suffix clears it.
    if RESERVED.contains(&out.as_str()) {
        out.push_str("-node");
    }

    Ok(out)
}

/// Whether a string is already a valid slug. Used to validate what we read back
/// off disk, since a human or another tool may have renamed a file.
pub fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_LEN
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !RESERVED.contains(&s)
}

/// Append a numeric suffix until the slug is unique among `taken`.
pub fn unique_slug(base: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    for n in 2..10_000 {
        let candidate = format!("{base}-{n}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    // Practically unreachable; a ULID suffix is always unique.
    format!("{base}-{}", crate::new_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_names() {
        assert_eq!(slugify("Kael Vantris").unwrap(), "kael-vantris");
        assert_eq!(slugify("Cinder Bay").unwrap(), "cinder-bay");
        assert_eq!(slugify("Ashglass Lantern").unwrap(), "ashglass-lantern");
    }

    #[test]
    fn strips_the_windows_forbidden_set() {
        // < > : " | ? * are illegal on NTFS and over SMB.
        assert_eq!(slugify(r#"a<b>c:d"e|f?g*h"#).unwrap(), "a-b-c-d-e-f-g-h");
    }

    #[test]
    fn never_leaves_a_trailing_dot_or_space() {
        // Windows silently strips these, which would desync the index from disk.
        assert_eq!(slugify("Ember Guild. ").unwrap(), "ember-guild");
        assert_eq!(slugify("  spaced  ").unwrap(), "spaced");
    }

    #[test]
    fn escapes_reserved_device_names() {
        assert_eq!(slugify("CON").unwrap(), "con-node");
        assert_eq!(slugify("com1").unwrap(), "com1-node");
        // Only exact matches are reserved.
        assert_eq!(slugify("Contour").unwrap(), "contour");
    }

    #[test]
    fn collapses_runs_of_separators() {
        assert_eq!(slugify("a  --  b").unwrap(), "a-b");
        assert!(is_valid_slug(&slugify("a  --  b").unwrap()));
    }

    #[test]
    fn truncates_without_leaving_a_dangling_separator() {
        let long = "word ".repeat(40);
        let slug = slugify(&long).unwrap();
        assert!(slug.len() <= MAX_LEN);
        assert!(is_valid_slug(&slug), "{slug} should still be valid");
    }

    #[test]
    fn rejects_names_with_nothing_usable() {
        assert!(slugify("日本語").is_err());
        assert!(slugify("!!!").is_err());
        assert!(slugify("").is_err());
    }

    #[test]
    fn unique_slug_walks_past_collisions() {
        let taken = |s: &str| matches!(s, "vashk" | "vashk-2");
        assert_eq!(unique_slug("vashk", &taken), "vashk-3");
        assert_eq!(unique_slug("sunborn", &taken), "sunborn");
    }
}
