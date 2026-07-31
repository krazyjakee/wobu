//! Splitting a Markdown file into YAML frontmatter and body.
//!
//! Deliberately dumb and line-based. The alternative — a real Markdown parser —
//! would let us round-trip more exotic files, but every byte we do not
//! understand is a byte we might drop when we rewrite the file, and these files
//! are meant to be safe to edit in Obsidian and then re-save from Wobu.

use std::path::Path;

use crate::error::{Error, Result};

const FENCE: &str = "---";

/// The two halves of a node file.
#[derive(Debug, Clone, PartialEq)]
pub struct Split<'a> {
    pub yaml: &'a str,
    pub body: &'a str,
}

/// Separate `---\n<yaml>\n---\n<body>`.
///
/// `path` is only used to name the file in errors.
pub fn split<'a>(path: &Path, text: &'a str) -> Result<Split<'a>> {
    // A UTF-8 BOM survives a round trip through Notepad and would otherwise
    // stop the opening fence from matching.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix(FENCE)
        .and_then(|r| r.strip_prefix('\n').or_else(|| r.strip_prefix("\r\n")))
        .ok_or_else(|| Error::MissingFrontmatter(path.to_path_buf()))?;

    // Find the closing fence: a line that is exactly `---`.
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == FENCE {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Ok(Split { yaml, body });
        }
        offset += line.len();
    }

    Err(Error::MissingFrontmatter(path.to_path_buf()))
}

/// Emit `---\n<yaml>---\n<body>`. `yaml` is expected to already end in a
/// newline, which is what `serde_norway` produces.
pub fn join(yaml: &str, body: &str) -> String {
    let mut out = String::with_capacity(yaml.len() + body.len() + 10);
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> &'static Path {
        Path::new("test.md")
    }

    #[test]
    fn splits_frontmatter_from_body() {
        let s = split(p(), "---\nid: abc\n---\n# Kael\n").unwrap();
        assert_eq!(s.yaml, "id: abc\n");
        assert_eq!(s.body, "# Kael\n");
    }

    #[test]
    fn tolerates_crlf_and_a_bom() {
        let s = split(p(), "\u{feff}---\r\nid: abc\r\n---\r\nbody\r\n").unwrap();
        assert_eq!(s.yaml, "id: abc\r\n");
        assert_eq!(s.body, "body\r\n");
    }

    #[test]
    fn a_horizontal_rule_in_the_body_is_not_the_closing_fence() {
        // The first bare `---` closes the frontmatter; later ones belong to the
        // body and must survive.
        let s = split(p(), "---\nid: abc\n---\nintro\n\n---\n\nmore\n").unwrap();
        assert_eq!(s.yaml, "id: abc\n");
        assert_eq!(s.body, "intro\n\n---\n\nmore\n");
    }

    #[test]
    fn a_file_with_no_frontmatter_is_rejected() {
        assert!(matches!(split(p(), "# Kael\n"), Err(Error::MissingFrontmatter(_))));
    }

    #[test]
    fn an_unterminated_fence_is_rejected() {
        assert!(matches!(split(p(), "---\nid: abc\n"), Err(Error::MissingFrontmatter(_))));
    }

    #[test]
    fn join_and_split_round_trip() {
        let text = join("id: abc\nkind: character\n", "## Notes\n\nscarred\n");
        let s = split(p(), &text).unwrap();
        assert_eq!(s.yaml, "id: abc\nkind: character\n");
        assert_eq!(s.body, "## Notes\n\nscarred\n");
    }
}
