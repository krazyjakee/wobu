//! Stripping credentials out of anything on its way to the webview.
//!
//! Keys live in the Rust process and the OS keychain. They are never sent to
//! the webview — but an error string is a side channel that carries them
//! anyway: a provider SDK that fails a request will happily put the full
//! `Authorization` header, or the request URL with `?key=…` still on it, into
//! its `Display` output. `docs/08-providers.md` treats that as a leak, so this
//! runs at the command boundary rather than at each call site: [`scrub`] is
//! applied inside `WobuError`'s constructor, which is the one path every error
//! crossing to the UI goes through. A crate cannot forget to call it.
//!
//! The bias is deliberately toward over-redaction. Losing a few characters of
//! a filename in a message is a cosmetic bug; leaking a key is not, and the
//! `detail` field exists for the technical remainder anyway.

pub const MASK: &str = "[redacted]";

/// Prefixes an issuer mints, which identify a credential on sight. Whatever
/// follows one of these is a secret regardless of how it looks.
///
/// Matched case-sensitively because that is how they are issued, and a
/// case-insensitive match on `sk-` would eat ordinary prose.
const ISSUER_PREFIXES: &[&str] = &[
    "sk-ant-", // Anthropic
    "sk-",     // OpenAI, and the many APIs that copied its shape
    "AIza",    // Google
    "ya29.",   // Google OAuth
    "ghp_",    // GitHub personal access
    "gho_",
    "github_pat_",
    "xoxb-", // Slack
    "xoxp-",
    "hf_",  // Hugging Face
    "AKIA", // AWS access key id
    "ASIA", // AWS temporary access key id
    "AKID", // Tencent secret id
    "r8_",  // Replicate
    "csk-",
];

/// Auth-scheme words. Unlike an issuer prefix these are ordinary English, so
/// "the Bearer token was rejected" must survive — what follows is only redacted
/// when it is shaped like a credential rather than like a word.
const SCHEME_PREFIXES: &[&str] = &["Bearer ", "Basic ", "Token "];

/// Names that make whatever follows them a secret. Matched case-insensitively
/// against `name = value`, `name: value` and `name=value` shapes.
///
/// Kept narrow on purpose: a bare `key` is only treated as a secret when it is
/// immediately assigned, because "no key for this provider" is a message we
/// want to keep readable.
const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "x-api-key",
    "secret_key",
    "secretkey",
    "secret_id",
    "secretid",
    "secret",
    "access_token",
    "refresh_token",
    "auth_token",
    "authorization",
    "password",
    "passwd",
    "token",
    "key",
];

/// True for the characters a credential is built from. Deliberately includes
/// the base64 alphabet's `+/=` so a padded token is consumed whole rather than
/// leaving its tail behind.
fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '=' | '~')
}

/// Whether a token is shaped like a credential rather than like a word.
///
/// Only consulted for the scheme prefixes, where the surrounding text is not
/// itself proof. Length alone settles most of it; the short-but-mixed case
/// catches things like `Basic YWxhZGRpbjE=`.
fn looks_like_credential(s: &str) -> bool {
    if s.len() >= 16 {
        return true;
    }
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    s.len() >= 8 && has_digit && has_alpha
}

/// Remove anything that looks like a credential.
///
/// Prefixes run before assignments, and the order is load-bearing:
/// `Authorization: Bearer sk-…` has a secret *name* whose value is the word
/// `Bearer`, so masking by name first would produce
/// `Authorization: [redacted] sk-…` and leave the key sitting in the open.
/// Going the other way, the prefix pass consumes the whole token and the
/// assignment pass then finds `[redacted]`, which it leaves alone.
///
/// Idempotent, which matters because errors get wrapped and re-rendered on the
/// way up and this runs at every layer.
pub fn scrub(input: &str) -> String {
    let out = scrub_prefixes(input);
    scrub_assignments(&out)
}

/// `api_key=sk-abc`, `Authorization: Bearer abc`, `secret = "abc"`.
fn scrub_assignments(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    'outer: while i < bytes.len() {
        for name in SECRET_NAMES {
            if !lower[i..].starts_with(name) {
                continue;
            }
            // Must be a whole word, or `mykey=` and `monkey=` would match.
            let before_ok = i == 0 || !is_name_char(bytes[i - 1] as char);
            if !before_ok {
                continue;
            }

            let after_name = i + name.len();
            let Some(value_start) = value_start_after(input, after_name) else { continue };

            out.push_str(&input[i..value_start]);
            let value_end = end_of_value(input, value_start);
            if value_end > value_start {
                out.push_str(MASK);
                i = value_end;
            } else {
                i = value_start;
            }
            continue 'outer;
        }
        // Not a secret name here — copy this character and move on. Indexed by
        // char so multi-byte content (a path with an em dash in it) survives.
        let ch = input[i..].chars().next().expect("index is on a char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Where the value begins after a secret name, skipping the separator and any
/// opening quote. `None` when what follows is not an assignment at all — which
/// is what keeps "no key for this provider" intact.
fn value_start_after(input: &str, mut i: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut seen_separator = false;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            // The quote closing the *name*, as in `{"api_key": "…"}`. Skipped
            // on both sides of the separator, since JSON puts one on each.
            b'"' | b'\'' | b'`' => {
                i += 1;
                if seen_separator {
                    return Some(i);
                }
            }
            b'=' | b':' if !seen_separator => {
                seen_separator = true;
                i += 1;
            }
            _ if seen_separator => return Some(i),
            _ => return None,
        }
    }
    None
}

fn end_of_value(input: &str, start: usize) -> usize {
    input[start..]
        .find(|c: char| !is_secret_char(c))
        .map(|offset| start + offset)
        .unwrap_or(input.len())
}

/// `sk-ant-api03-…` and friends, wherever they appear — including inside a URL
/// or a JSON blob, where there is no `name=` to key off.
fn scrub_prefixes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    'scan: loop {
        // The earliest prefix wins, and at equal position the longest, so
        // `sk-ant-` is not pre-empted by `sk-` matching at the same index and
        // leaving `ant-…` behind.
        let mut best: Option<(usize, usize, bool)> = None;
        for (prefix, issuer) in ISSUER_PREFIXES
            .iter()
            .map(|p| (p, true))
            .chain(SCHEME_PREFIXES.iter().map(|p| (p, false)))
        {
            let Some(at) = rest.find(*prefix) else { continue };
            let better = match best {
                None => true,
                Some((best_at, best_len, _)) => at < best_at || (at == best_at && prefix.len() > best_len),
            };
            if better {
                best = Some((at, prefix.len(), issuer));
            }
        }

        let Some((at, len, issuer)) = best else { break 'scan };
        let value_start = at + len;
        let value_end = end_of_value(rest, value_start);
        let value = &rest[value_start..value_end];

        // An issuer prefix is proof on its own; a scheme word is not, so
        // "the Bearer token was rejected" has to get past here unchanged.
        let redact_it = !value.is_empty() && (issuer || looks_like_credential(value));

        out.push_str(&rest[..value_start]);
        if redact_it {
            out.push_str(MASK);
            rest = &rest[value_end..];
        } else {
            // Step past the prefix only, so a later prefix inside the same
            // run still gets its turn.
            rest = &rest[value_start..];
        }
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test the issue asks for by name: a provider error carrying a key
    /// must not carry it any further.
    #[test]
    fn a_provider_error_does_not_leak_the_key() {
        let raw = "request failed: POST https://api.anthropic.com/v1/messages \
                   (x-api-key: sk-ant-api03-Zm9vYmFyYmF6cXV1eA-AA) returned 401";
        let safe = scrub(raw);

        assert!(!safe.contains("sk-ant-api03-Zm9vYmFyYmF6cXV1eA-AA"), "{safe}");
        assert!(!safe.contains("Zm9vYmFyYmF6cXV1eA"), "{safe}");
        // Still diagnosable: the endpoint and the status survive.
        assert!(safe.contains("api.anthropic.com"), "{safe}");
        assert!(safe.contains("401"), "{safe}");
    }

    #[test]
    fn keys_are_caught_by_prefix_wherever_they_sit() {
        for raw in [
            "sk-ant-api03-abcdefghijklmnop",
            "openai said no to sk-proj-abcdefghijklmnop",
            "https://generativelanguage.googleapis.com/v1/models?key=AIzaSyD-abcdefghijk",
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.abc",
            "aws AKIAIOSFODNN7EXAMPLE rejected",
            "token ghp_16C7e42F292c6912E7710c838347Ae178B4a",
        ] {
            let safe = scrub(raw);
            assert!(safe.contains(MASK), "nothing redacted in: {safe}");
            for leak in ["abcdefghijklmnop", "AIzaSyD-abcdefghijk", "eyJzdWIiOiIxIn0", "IOSFODNN7EXAMPLE", "16C7e42F292c6912E7710c838347Ae178B4a"] {
                assert!(!safe.contains(leak), "leaked `{leak}` in: {safe}");
            }
        }
    }

    #[test]
    fn keys_are_caught_by_name_even_with_an_unknown_prefix() {
        // A backend we have never seen, whose keys look like nothing in
        // SECRET_PREFIXES. The `name =` shape is the only signal.
        let safe = scrub(r#"{"api_key": "wobbly-9f8e7d6c5b4a", "model": "flux-dev"}"#);
        assert!(!safe.contains("wobbly-9f8e7d6c5b4a"), "{safe}");
        assert!(safe.contains("flux-dev"), "the non-secret field should survive: {safe}");

        let safe = scrub("SecretId=AKIDzzzzzzzz SecretKey=Gu5t9xzzzzzz region=ap-guangzhou");
        assert!(!safe.contains("AKIDzzzzzzzz"), "{safe}");
        assert!(!safe.contains("Gu5t9xzzzzzz"), "{safe}");
        assert!(safe.contains("ap-guangzhou"), "{safe}");
    }

    #[test]
    fn ordinary_messages_are_left_alone() {
        // Every one of these is a real message from wobu-store or wobu-core.
        for raw in [
            "no node with id 01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "/mnt/share/Ashfall.wobu is not a Wobu project (no project.json)",
            "the project folder is read-only",
            "`character` nodes cannot nest inside one another",
            "moving `Cinder Bay` under `Cinder Bay` would create a cycle",
        ] {
            assert_eq!(scrub(raw), raw, "message was altered");
        }
    }

    #[test]
    fn no_key_configured_stays_readable() {
        // The failure mode that matters: over-redaction turning the one error
        // that tells a user what to do into `[redacted]`.
        let raw = "no key for this provider — add one in Settings";
        assert_eq!(scrub(raw), raw);
        assert_eq!(scrub("the Bearer token was rejected"), "the Bearer token was rejected");
    }

    #[test]
    fn scrubbing_is_idempotent() {
        // Errors get wrapped and re-rendered on the way up, so this runs more
        // than once over the same string.
        let once = scrub("api_key=sk-ant-api03-abcdefghijkl failed");
        assert_eq!(scrub(&once), once, "second pass changed the result: {once}");
    }

    #[test]
    fn multibyte_text_survives() {
        // wobu-store's messages contain em dashes, and indexing by byte
        // through one would panic.
        let raw = "malformed node file /world/Ashfall.wobu/nodes/species/vashk.md: \
                   expected a mapping — found a sequence";
        assert_eq!(scrub(raw), raw);
    }
}
