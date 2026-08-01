//! TC3-HMAC-SHA256: turning a `SecretId`/`SecretKey` pair into an
//! `Authorization` header Tencent will accept.
//!
//! Everything here is a pure function of strings and a timestamp, so the step
//! that costs a day to debug can be driven from published vectors instead of
//! from a live account — the same split as `gemini/wire.rs` and `comfy/wire.rs`,
//! applied to the one provider where the *request* rather than the response is
//! the part that is easy to get wrong.
//!
//! Tencent has no bearer key. Authentication is an AWS-SigV4-shaped
//! canonical-request construction, and every way of getting it wrong produces
//! the same answer: `AuthFailure.SignatureFailure`, with no indication of which
//! of the four steps disagreed. That is why each step below is asserted
//! separately rather than only the signature at the end — a test that says
//! "the signature is wrong" tells the next person nothing they did not already
//! know from the 403.
//!
//! ## Where the vectors came from
//!
//! Tencent's own walkthrough, read on **2026-08-01** from the signature chapter
//! that is reprinted verbatim in every product's API PDF; the copy checked was
//! the Hunyuan 3D Global one, which is the product we call:
//!
//! - <https://www.tencentcloud.com/document/product/845/32207> ("TC3-HMAC-SHA256
//!   Signature Algorithm")
//! - <https://staticintl.cloudcachetci.com/doc/pdf/product/pdf/1281_74103_en.pdf>
//!   (Tencent Hunyuan 3D Global, pages 16-21 — the same text, and the copy the
//!   constants below were transcribed from because it can be downloaded and
//!   diffed rather than rendered by JavaScript)
//!
//! **The published example masks the credentials** — `SecretId` is printed as
//! `AKID********************************` and `SecretKey` as thirty-two
//! asterisks. So the intermediates split in two:
//!
//! - **Steps 1 and 2 are Tencent's published values and are reproducible**: the
//!   hashed payload, the canonical request, its hash, and the string to sign
//!   depend on nothing secret. Those are asserted against the doc's own text
//!   character for character.
//! - **Steps 3 and 4 cannot be reproduced from the doc**, because the derived
//!   keys and the final signature depend on the `SecretKey` that was masked out.
//!   Two other mirrors of the same walkthrough
//!   (<https://edgeone.ai/document/50458>, <https://docs.cloudbase.net/en/http-api/basic/tc3-auth>)
//!   were checked on the same day and mask it identically, and no archived
//!   unmasked copy was reachable. So the HMAC chain is pinned three ways
//!   instead: against RFC 4231's HMAC-SHA256 vectors (which prove the primitive
//!   and its wiring), against the chain recomputed inline from literals inside
//!   the test (which proves the order and the `tc3_request` terminator), and
//!   against a fixture whose expected values are **ours, not Tencent's** and are
//!   labelled as such wherever they appear.
//!
//! One more thing the doc's example is not: it signs `content-type;host`, and we
//! sign `content-type;host;x-tc-action`. `docs/08-providers.md` records the
//! three-header set as the one verified against a live account on 2026-07-31, so
//! that is what [`sign`] emits; the two-header example is still the right vector
//! for [`canonical_request`], which is why that function takes its headers
//! rather than knowing them.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::error::Error;

/// The name that reaches the user in an error message.
pub(crate) const BACKEND: &str = "Tencent Hunyuan3D";

/// Fixed by Tencent, and part of both the string to sign and the
/// `Authorization` header.
const ALGORITHM: &str = "TC3-HMAC-SHA256";

/// The credential-scope terminator. An underscore, not a hyphen — the one
/// literal in the whole construction with no error message of its own.
const TERMINATOR: &str = "tc3_request";

/// The `Content-Type` the signature is computed over, and therefore the one the
/// request must actually carry.
///
/// Exported rather than inlined because the doc warns about exactly this: "in
/// some programming languages, a charset value would be added even if it is not
/// specified. In this case, the request sent is different from the one signed,
/// and the server will return an error indicating that signature verification
/// failed." An adapter that lets its HTTP client pick the header while this file
/// signs a different one produces an auth failure that looks like a bad key.
pub const CONTENT_TYPE: &str = "application/json; charset=utf-8";

/* ── the credential ───────────────────────────────────────────────────────── */

/// The account-wide master secret, in the one shape that cannot be printed by
/// accident.
///
/// The same arrangement as `Secret` in `src-tauri/src/keys.rs`, and it has to be
/// made again here rather than reused because that type lives in the Tauri shell
/// and this crate cannot depend on the shell. The reasoning carries over
/// unchanged and matters more, not less: `docs/08-providers.md` notes that a
/// Tencent `SecretKey` is "an account-wide master credential, not a scoped
/// token — materially more dangerous to hold than an OpenAI-style key".
///
/// No `Display`, no `Serialize`, and a hand-written [`std::fmt::Debug`]. A
/// derived `Debug` is the classic leak, because every type that ends up holding
/// one of these derives `Debug` and one `{:?}` in a panic message is enough.
#[derive(Clone)]
pub struct SecretKey(String);

impl SecretKey {
    pub fn new(value: impl Into<String>) -> SecretKey {
        SecretKey(value.into())
    }
}

/// The same mask `redact::MASK` uses in the shell, so a line reads identically
/// whether it was scrubbed on the way out or was never printable to begin with.
const MASK: &str = "[redacted]";

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(MASK)
    }
}

/// A Tencent Cloud key pair.
///
/// The `SecretId` is masked in `Debug` alongside the key even though it travels
/// in cleartext in every `Authorization` header we send. It is not a secret, but
/// it does name the account, and Tencent's own documentation prints it as
/// `AKID********************************` — there is no diagnostic worth having
/// that needs it in a log line.
#[derive(Clone)]
pub struct Credentials {
    pub secret_id: String,
    pub secret_key: SecretKey,
}

impl Credentials {
    pub fn new(secret_id: impl Into<String>, secret_key: SecretKey) -> Credentials {
        Credentials { secret_id: secret_id.into(), secret_key }
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").field("secret_id", &MASK).field("secret_key", &MASK).finish()
    }
}

/* ── the call being signed ────────────────────────────────────────────────── */

/// One TencentCloud API call, in the terms the signature is computed over.
///
/// Every field here is either signed or part of the credential scope, which is
/// why they are gathered rather than passed loose: `service` and `host` have to
/// agree (the scope names the service, the signature covers the host), and
/// `region` has to be the same on the poll as on the submit
/// (`docs/08-providers.md`). A struct makes both visible at the call site.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// `hunyuan.intl.tencentcloudapi.com` for us. Signed.
    pub host: &'a str,
    /// The product name, and the middle segment of the credential scope. Must
    /// match the host's leading label — `hunyuan`, not `ai3d`.
    pub service: &'a str,
    /// `SubmitHunyuanTo3DProJob` or `QueryHunyuanTo3DProJob`. Signed, and signed
    /// **lowercased**, while the header itself is sent in its documented case.
    pub action: &'a str,
    /// `2023-09-01` for the international Hunyuan namespace. Sent unsigned.
    pub version: &'a str,
    /// One of the three regions the international endpoint accepts. Sent
    /// unsigned.
    pub region: &'a str,
    /// The JSON body, exactly as it will be written to the socket. Hashed, so a
    /// body that is re-serialised between here and the send is a signature
    /// failure.
    pub body: &'a str,
}

/// The headers to send, `Authorization` included.
///
/// `Debug` prints names only. The `Authorization` value carries the `SecretId`
/// and a valid signature for a five-minute window, and an adapter that dumps its
/// outbound headers into an error is the realistic way both reach a log file.
#[derive(Clone)]
pub struct Signed {
    headers: Vec<(&'static str, String)>,
}

impl Signed {
    /// Every header to put on the request, in the order Tencent's own example
    /// lists them.
    pub fn headers(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.headers.iter().map(|(name, value)| (*name, value.as_str()))
    }

    /// The `Authorization` value on its own, for a client that sets it apart
    /// from the rest.
    pub fn authorization(&self) -> &str {
        self.headers
            .iter()
            .find(|(name, _)| *name == "Authorization")
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for Signed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signed")
            .field("headers", &self.headers.iter().map(|(name, _)| *name).collect::<Vec<_>>())
            .finish()
    }
}

/* ── signing ──────────────────────────────────────────────────────────────── */

/// Sign a call, at a time the caller chooses.
///
/// **`timestamp` is a parameter and not a `SystemTime::now()` inside.** A
/// function that reads the clock cannot be checked against a fixed vector, and
/// this is the function that most needs checking against one. The adapter passes
/// the real clock; the tests pass Tencent's.
///
/// Seconds since the Unix epoch, UTC. The date in the credential scope is
/// derived from it here rather than taken as a second argument, because the two
/// disagreeing is the failure Tencent's own note calls out: a caller that
/// formats the date in local time signs successfully all day and fails every
/// night at midnight, which is the least diagnosable version of this bug there
/// is.
pub fn sign(call: &Call<'_>, credentials: &Credentials, timestamp: i64) -> Signed {
    // Lowercased for the signature only. Tencent lowercases header *values* as
    // well as names when building the canonical form, and `X-TC-Action` is the
    // one signed header whose value is not already lowercase — sending
    // `submithunyuanto3dprojob` in the actual header is a `InvalidAction`.
    let action_header = call.action.to_ascii_lowercase();
    let headers = [
        ("content-type", CONTENT_TYPE),
        ("host", call.host),
        ("x-tc-action", action_header.as_str()),
    ];

    let canonical = canonical_request("POST", "/", "", &headers, call.body);
    let signed_headers = signed_headers(&headers);
    let date = utc_date(timestamp);
    let scope = format!("{date}/{}/{TERMINATOR}", call.service);
    let to_sign = string_to_sign(timestamp, &scope, &canonical);
    let signature =
        hex(&hmac(&signing_key(&credentials.secret_key, &date, call.service), &to_sign));

    let authorization = format!(
        "{ALGORITHM} Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.secret_id,
    );

    Signed {
        headers: vec![
            ("Authorization", authorization),
            ("Content-Type", CONTENT_TYPE.to_owned()),
            ("Host", call.host.to_owned()),
            ("X-TC-Action", call.action.to_owned()),
            // The three unsigned ones. They are common parameters rather than
            // part of the signature, but omitting any of them is a rejection
            // that looks nothing like the header that is missing.
            ("X-TC-Timestamp", timestamp.to_string()),
            ("X-TC-Version", call.version.to_owned()),
            ("X-TC-Region", call.region.to_owned()),
        ],
    }
}

/// Step 1. The canonical request.
///
/// Takes its headers rather than knowing them so that Tencent's published
/// two-header example is expressible; [`sign`] is the only caller that decides
/// which three we sign.
///
/// The shape, from the doc:
///
/// ```text
/// HTTPRequestMethod + '\n' + CanonicalURI + '\n' + CanonicalQueryString + '\n'
///     + CanonicalHeaders + '\n' + SignedHeaders + '\n' + HashedRequestPayload
/// ```
///
/// `CanonicalHeaders` itself ends with a newline, so the `'\n'` after it lands as
/// a blank line. That blank line is the single most common way to get this wrong
/// and it has no error message.
fn canonical_request(
    method: &str,
    uri: &str,
    query: &str,
    headers: &[(&str, &str)],
    payload: &str,
) -> String {
    let mut sorted: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_ascii_lowercase()))
        .collect();
    // ASCII ascending by the lowercased name. Sorting the original case would
    // put `X-TC-Action` before `content-type`, which is a different — valid
    // looking, and rejected — canonical request.
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers: String =
        sorted.iter().map(|(name, value)| format!("{name}:{value}\n")).collect();
    let signed: Vec<(&str, &str)> = sorted.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();

    format!(
        "{method}\n{uri}\n{query}\n{canonical_headers}\n{}\n{}",
        signed_headers(&signed),
        sha256_hex(payload.as_bytes()),
    )
}

/// The `SignedHeaders` list: lowercased names, ASCII ascending, semicolon
/// separated.
///
/// Computed from the same slice the canonical headers are built from, so the two
/// cannot fall out of step — the doc requires that they "each individually
/// correspond".
fn signed_headers(headers: &[(&str, &str)]) -> String {
    let mut names: Vec<String> =
        headers.iter().map(|(name, _)| name.trim().to_ascii_lowercase()).collect();
    names.sort();
    names.join(";")
}

/// Step 2. The string to sign.
fn string_to_sign(timestamp: i64, scope: &str, canonical: &str) -> String {
    format!("{ALGORITHM}\n{timestamp}\n{scope}\n{}", sha256_hex(canonical.as_bytes()))
}

/// Step 3. The derived signing key.
///
/// ```text
/// SecretDate    = HMAC_SHA256("TC3" + SecretKey, Date)
/// SecretService = HMAC_SHA256(SecretDate, Service)
/// SecretSigning = HMAC_SHA256(SecretService, "tc3_request")
/// ```
///
/// Three chained HMACs where the *output* of each is the *key* of the next, and
/// the message is the scope segment. Swapping key and message at any link
/// produces a well-formed 32-byte key and a signature Tencent will not accept.
fn signing_key(secret_key: &SecretKey, date: &str, service: &str) -> [u8; 32] {
    let secret_date = hmac(format!("TC3{}", secret_key.0).as_bytes(), date);
    let secret_service = hmac(&secret_date, service);
    hmac(&secret_service, TERMINATOR)
}

fn hmac(key: &[u8], message: &str) -> [u8; 32] {
    let mut mac = <Hmac<Sha256>>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts a key of any length, so this cannot fail");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().into()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    // Lowercase, which the doc spells out twice as `Lowercase(HexEncode(...))`.
    // A table rather than `format!("{byte:02x}")` per byte so that the case is a
    // property of the data and cannot be changed by a format-string edit.
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// The UTC calendar date of a Unix timestamp, as `YYYY-MM-DD`.
///
/// Hand-written rather than pulled in with a date crate: this is the only date
/// arithmetic in the crate, and the alternative is a dependency whose whole
/// surface is time zones we must not use. The doc is explicit that using a local
/// zone here "can succeed both day and night but will definitely fail at 00:00",
/// so the safest implementation is the one that has no zone to get wrong.
///
/// `civil_from_days`, from Howard Hinnant's `chrono`-compatible date algorithms —
/// exact for the whole range of `i64` seconds, with March as the first month of
/// the internal year so that the leap day falls at the end and needs no special
/// case.
fn utc_date(timestamp: i64) -> String {
    // Euclidean rather than truncating: a negative timestamp must round *down*
    // to the previous day, not toward zero.
    let days = timestamp.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/* ── what Tencent says when the signature is not accepted ─────────────────── */

/// The `AuthFailure.*` family, mapped to something the user can act on.
///
/// Tencent reports these inside a **200** with an `Error.Code` in the body, so
/// there is no status code to read; the string is the whole of the signal.
/// Returns `None` for anything outside the family, because the rest of the error
/// surface belongs to the adapter
/// ([#64](https://github.com/krazyjakee/wobu/issues/64)) and a signing module
/// that claimed `FailedOperation.ServiceNotActivated` would be answering a
/// question it cannot see.
///
/// The split that matters is `SignatureExpire` against everything else. Both are
/// `AuthFailure`, both arrive identically, and they send the user to two
/// completely different places — one to Settings to repaste a key, the other to
/// the operating system's date-and-time panel. Reporting the second as the first
/// is a user deleting a working credential.
pub fn auth_failure(code: &str) -> Option<Error> {
    match code {
        // Five minutes of drift, which a desktop reaches on its own. This is the
        // named mapping `docs/08-providers.md` asks for.
        "AuthFailure.SignatureExpire" => Some(Error::ClockSkew { backend: BACKEND }),

        // The rest are a credential problem from the user's side of the line.
        // `SignatureFailure` is the ambiguous one — it is also what *our* bug
        // looks like — and it is reported as a key problem anyway, because the
        // vectors in this file are what stands behind that choice: if they pass,
        // the construction is right and the key is the remaining variable.
        "AuthFailure.SignatureFailure"
        | "AuthFailure.SecretIdNotFound"
        | "AuthFailure.InvalidSecretId"
        | "AuthFailure.TokenFailure"
        | "AuthFailure.MFAFailure"
        | "AuthFailure.UnauthorizedOperation" => Some(Error::BadKey { backend: BACKEND }),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* ── Tencent's published example ──────────────────────────────────────
     *
     * Transcribed on 2026-08-01 from the signature chapter of
     * <https://staticintl.cloudcachetci.com/doc/pdf/product/pdf/1281_74103_en.pdf>
     * (pages 16-21), which is the same text as
     * <https://www.tencentcloud.com/document/product/845/32207>.
     *
     * Everything in this block is Tencent's, character for character. The
     * example signs `content-type;host` — two headers, not our three — and is
     * kept that way on purpose: changing it to match our request would mean the
     * expected values were ours rather than theirs, and the whole value of the
     * vector is that it is not.
     */

    const DOC_PAYLOAD: &str =
        r#"{"Limit": 1, "Filters": [{"Values": ["unnamed"], "Name": "instance-name"}]}"#;
    const DOC_HASHED_PAYLOAD: &str =
        "99d58dfbc6745f6747f36bfca17dee5e6881dc0428a0a36f96199342bc5b4907";
    const DOC_CANONICAL_REQUEST: &str = "POST\n\
        /\n\
        \n\
        content-type:application/json; charset=utf-8\n\
        host:cvm.tencentcloudapi.com\n\
        \n\
        content-type;host\n\
        99d58dfbc6745f6747f36bfca17dee5e6881dc0428a0a36f96199342bc5b4907";
    const DOC_HASHED_CANONICAL_REQUEST: &str =
        "2815843035062fffda5fd6f2a44ea8a34818b0dc46f024b8b3786976a3adda7a";
    const DOC_STRING_TO_SIGN: &str = "TC3-HMAC-SHA256\n\
        1551113065\n\
        2019-02-25/cvm/tc3_request\n\
        2815843035062fffda5fd6f2a44ea8a34818b0dc46f024b8b3786976a3adda7a";
    const DOC_TIMESTAMP: i64 = 1551113065;

    fn doc_headers() -> Vec<(&'static str, &'static str)> {
        vec![("content-type", CONTENT_TYPE), ("host", "cvm.tencentcloudapi.com")]
    }

    #[test]
    fn step_one_reproduces_tencents_published_canonical_request_byte_for_byte() {
        // The regression: any edit to the concatenation — a lost blank line, a
        // stray trailing newline, a `\r\n` — that still produces a plausible
        // string. Asserted whole rather than field by field because the bug is
        // always in the joins, and this is the only place a whole one exists
        // that we did not write ourselves.
        let built = canonical_request("POST", "/", "", &doc_headers(), DOC_PAYLOAD);
        assert_eq!(built, DOC_CANONICAL_REQUEST);
    }

    #[test]
    fn step_one_hashes_the_body_and_not_the_request() {
        // `HashedRequestPayload` is the last line, and it is the hash of the
        // body alone. Hashing the whole request here instead still produces 64
        // hex characters in the right place.
        assert_eq!(sha256_hex(DOC_PAYLOAD.as_bytes()), DOC_HASHED_PAYLOAD);
        assert!(
            canonical_request("POST", "/", "", &doc_headers(), DOC_PAYLOAD)
                .ends_with(DOC_HASHED_PAYLOAD)
        );
    }

    #[test]
    fn step_two_reproduces_tencents_published_string_to_sign() {
        // Covers the hash of step 1's output as well: `2815843035...` is
        // Tencent's, so a canonical request that differs from theirs by one byte
        // fails here even if step 1 were somehow asserted loosely.
        let canonical = canonical_request("POST", "/", "", &doc_headers(), DOC_PAYLOAD);
        assert_eq!(sha256_hex(canonical.as_bytes()), DOC_HASHED_CANONICAL_REQUEST);

        let built = string_to_sign(DOC_TIMESTAMP, "2019-02-25/cvm/tc3_request", &canonical);
        assert_eq!(built, DOC_STRING_TO_SIGN);
    }

    #[test]
    fn the_credential_scope_date_is_utc_and_not_the_machines_own_zone() {
        // Tencent's note, verbatim: "if the timestamp is 1551113065 and the time
        // in UTC+8 is 2019-02-26 00:44:25, the UTC+0 date in the calculated Date
        // value should be 2019-02-25 instead of 2019-02-26". A local-time
        // implementation passes every test written at noon and fails every night
        // at midnight, which is the failure this exists to make loud.
        assert_eq!(utc_date(1551113065), "2019-02-25");
        assert_eq!(utc_date(1551139199), "2019-02-25", "the last second of the UTC day");
        assert_eq!(utc_date(1551139200), "2019-02-26", "the first second of the next");
    }

    #[test]
    fn the_date_arithmetic_survives_leap_days_and_century_rules() {
        // `civil_from_days` is hand-written, so the three cases that break naive
        // versions are pinned: the epoch itself, a leap day in a year divisible
        // by 400, and the day after the February of a leap year that a
        // divisible-by-100 rule would have got wrong.
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(utc_date(1_583_020_800), "2020-03-01");
        assert_eq!(utc_date(4_102_444_800), "2100-01-01");
        // Before the epoch, where a truncating division rounds the wrong way.
        assert_eq!(utc_date(-1), "1969-12-31");
    }

    /* ── the canonical request, assertion by assertion ────────────────────
     *
     * Each of these is one documented rule. They exist separately from the
     * whole-string comparison above because that comparison only ever says
     * "different", and there are seven distinct ways to be different.
     */

    #[test]
    fn header_names_and_values_are_both_lowercased_for_the_signature() {
        // Tencent lowercases the value as well as the name, which is easy to
        // miss because every header in their example is already lowercase.
        // `X-TC-Action` is the one where it shows: the signature covers
        // `submithunyuanto3dprojob` while the wire carries the documented case.
        let built = canonical_request(
            "POST",
            "/",
            "",
            &[("Content-Type", CONTENT_TYPE), ("Host", "H.example"), ("X-TC-Action", "DoAThing")],
            "",
        );
        assert!(built.contains("\nx-tc-action:doathing\n"), "{built}");
        assert!(built.contains("\nhost:h.example\n"), "{built}");
        assert!(!built.contains("X-TC-Action"), "{built}");
    }

    #[test]
    fn headers_are_sorted_by_their_lowercased_name_and_not_by_the_order_given() {
        // Sorting the original case puts every `X-`prefixed header first,
        // because uppercase `X` sorts before lowercase `c`. That produces a
        // canonical request that looks entirely reasonable and is rejected.
        let scrambled = canonical_request(
            "POST",
            "/",
            "",
            &[("X-TC-Action", "Act"), ("host", "h.example"), ("Content-Type", CONTENT_TYPE)],
            "",
        );
        let ordered = canonical_request(
            "POST",
            "/",
            "",
            &[("content-type", CONTENT_TYPE), ("host", "h.example"), ("x-tc-action", "act")],
            "",
        );
        assert_eq!(scrambled, ordered);
        assert!(scrambled.contains("\ncontent-type;host;x-tc-action\n"), "{scrambled}");
    }

    #[test]
    fn the_canonical_headers_block_ends_with_a_newline_and_is_followed_by_a_blank_line() {
        // The blank line between the headers and `SignedHeaders` comes from the
        // '\n' the format adds *after* a block that already ends in one. Drop
        // either and the string is one character shorter with no other symptom.
        let built = canonical_request("POST", "/", "", &doc_headers(), DOC_PAYLOAD);
        let lines: Vec<&str> = built.split('\n').collect();
        assert_eq!(lines[0], "POST");
        assert_eq!(lines[1], "/");
        assert_eq!(lines[2], "", "the canonical query string, empty for a POST");
        assert_eq!(lines[3], "content-type:application/json; charset=utf-8");
        assert_eq!(lines[4], "host:cvm.tencentcloudapi.com");
        assert_eq!(lines[5], "", "the blank line the headers block's own newline creates");
        assert_eq!(lines[6], "content-type;host");
        assert_eq!(lines[7], DOC_HASHED_PAYLOAD);
        assert_eq!(lines.len(), 8, "an eighth newline would mean a trailing one");
    }

    #[test]
    fn the_canonical_request_has_no_trailing_newline() {
        // Its hash goes straight into the string to sign, so one extra byte here
        // changes `HashedCanonicalRequest` completely and nothing says so.
        let built = canonical_request("POST", "/", "", &doc_headers(), DOC_PAYLOAD);
        assert!(!built.ends_with('\n'), "{built:?}");
    }

    #[test]
    fn the_string_to_sign_has_no_trailing_newline_either() {
        // Same failure one step later, and this one is the HMAC's message
        // rather than a hash input, so there is not even a hex string to eyeball.
        let to_sign = string_to_sign(DOC_TIMESTAMP, "2019-02-25/cvm/tc3_request", "x");
        assert!(!to_sign.ends_with('\n'), "{to_sign:?}");
        assert_eq!(to_sign.matches('\n').count(), 3, "{to_sign:?}");
    }

    #[test]
    fn an_empty_body_hashes_to_the_sha256_of_the_empty_string_rather_than_being_skipped() {
        // "For GET requests, RequestPayload is always an empty string." An
        // implementation that omits the line for an empty body loses the final
        // newline as well, so this guards two things at once. The constant is
        // the SHA-256 of zero bytes and is not Tencent-specific.
        const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(sha256_hex(b""), EMPTY);
        let built = canonical_request("POST", "/", "", &doc_headers(), "");
        assert!(built.ends_with(&format!("\ncontent-type;host\n{EMPTY}")), "{built}");
    }

    #[test]
    fn the_signed_header_list_always_matches_the_headers_that_were_canonicalised() {
        // The doc requires the two to "each individually correspond". A list
        // naming a header the block does not carry — or carrying one the list
        // does not name — is a rejection with no hint which side is wrong.
        let headers =
            [("X-TC-Action", "Act"), ("host", "h.example"), ("Content-Type", CONTENT_TYPE)];
        let built = canonical_request("POST", "/", "", &headers, "");
        let lines: Vec<&str> = built.split('\n').collect();
        let block: Vec<&str> =
            lines[3..6].iter().map(|line| line.split(':').next().unwrap()).collect();
        assert_eq!(block.join(";"), signed_headers(&headers));
    }

    /* ── the HMAC chain ───────────────────────────────────────────────────
     *
     * Tencent masks the `SecretKey` in the published example, so steps 3 and 4
     * have no vector of theirs to check against. These check the primitive
     * against RFC 4231 and the chain against itself.
     */

    #[test]
    fn hmac_sha256_matches_the_rfc_4231_vectors() {
        // Not Tencent's, and deliberately so: if the derived-key test below
        // fails, this says whether the problem is the chain or the primitive
        // underneath it. RFC 4231 section 4.2 and 4.3.
        assert_eq!(
            hex(&hmac(&[0x0b; 20], "Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        );
        assert_eq!(
            hex(&hmac(b"Jefe", "what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        );
    }

    #[test]
    fn the_derived_key_chains_key_into_key_in_the_documented_order() {
        // The regression: swapping a key and a message, reordering date and
        // service, or writing the terminator as `tc3-request`. Every one of
        // those still yields 32 bytes. Recomputed here from literals rather than
        // compared to a stored hex string, so the *shape* is what is asserted
        // and a future refactor of `signing_key` cannot satisfy it by accident.
        let key = SecretKey::new("fixture-key");
        let expected =
            hmac(&hmac(&hmac(b"TC3fixture-key", "2026-02-01"), "hunyuan"), "tc3_request");
        assert_eq!(signing_key(&key, "2026-02-01", "hunyuan"), expected);

        // And the three ways to get it wrong, spelled out so a passing test
        // cannot be the result of an accidental symmetry.
        assert_ne!(
            signing_key(&key, "2026-02-01", "hunyuan"),
            hmac(&hmac(&hmac(b"TC3fixture-key", "hunyuan"), "2026-02-01"), "tc3_request"),
        );
        assert_ne!(
            signing_key(&key, "2026-02-01", "hunyuan"),
            hmac(&hmac(&hmac(b"fixture-key", "2026-02-01"), "hunyuan"), "tc3_request"),
        );
        assert_ne!(
            signing_key(&key, "2026-02-01", "hunyuan"),
            hmac(&hmac(&hmac(b"TC3fixture-key", "2026-02-01"), "hunyuan"), "tc3-request"),
        );
    }

    /* ── the whole thing, on a Hunyuan3D call ─────────────────────────────
     *
     * OURS, NOT TENCENT'S. The expected signature below was produced by an
     * independent Python implementation of the published pseudocode against the
     * fictional credentials here, because the doc's own worked example masks its
     * key and cannot be completed. It is a change detector for the assembly of
     * the four steps, not evidence that the four steps are right — the vectors
     * above are what carry that.
     */

    const FIXTURE_SECRET_ID: &str = "AKIDwobufixturewobufixturewobufixtur";
    const FIXTURE_SECRET_KEY: &str = "wobu-fixture-secret-key-not-a-real-one";
    const FIXTURE_TIMESTAMP: i64 = 1_769_904_000;
    const FIXTURE_BODY: &str = r#"{"Model":"3.1","Prompt":"a mossy stone lantern"}"#;
    const FIXTURE_SIGNATURE: &str =
        "42ad94abbfc6455e282c71161e8485bc6e26a2c74f44387205907625ff81f151";

    fn fixture() -> (Call<'static>, Credentials) {
        (
            Call {
                host: "hunyuan.intl.tencentcloudapi.com",
                service: "hunyuan",
                action: "SubmitHunyuanTo3DProJob",
                version: "2023-09-01",
                region: "ap-singapore",
                body: FIXTURE_BODY,
            },
            Credentials::new(FIXTURE_SECRET_ID, SecretKey::new(FIXTURE_SECRET_KEY)),
        )
    }

    #[test]
    fn a_hunyuan_call_signs_to_a_stable_authorization_header() {
        let (call, credentials) = fixture();
        let signed = sign(&call, &credentials, FIXTURE_TIMESTAMP);
        assert_eq!(
            signed.authorization(),
            format!(
                "TC3-HMAC-SHA256 Credential={FIXTURE_SECRET_ID}/2026-02-01/hunyuan/tc3_request, \
                 SignedHeaders=content-type;host;x-tc-action, Signature={FIXTURE_SIGNATURE}"
            ),
        );
    }

    #[test]
    fn the_authorization_header_uses_tencents_punctuation_and_not_amazons() {
        // The construction is SigV4-shaped and the header is not: Tencent uses
        // `, ` between the three parts and `=` inside them, where SigV4 uses
        // `,` with no space. Copying a SigV4 implementation gets this wrong and
        // the only symptom is an auth failure.
        let (call, credentials) = fixture();
        let header = sign(&call, &credentials, FIXTURE_TIMESTAMP).authorization().to_owned();
        assert!(header.starts_with("TC3-HMAC-SHA256 Credential="), "{header}");
        assert_eq!(header.matches(", ").count(), 2, "{header}");
        assert!(header.contains(", SignedHeaders=content-type;host;x-tc-action, Signature="));
    }

    #[test]
    fn exactly_three_headers_are_signed_and_three_more_ride_unsigned() {
        // `docs/08-providers.md` records this split as verified against a live
        // account on 2026-07-31. Signing `x-tc-timestamp` — which SigV4-shaped
        // implementations tend to do with `x-amz-date` — is a rejection, and
        // *omitting* an unsigned one is a different rejection.
        let (call, credentials) = fixture();
        let signed = sign(&call, &credentials, FIXTURE_TIMESTAMP);
        assert!(signed.authorization().contains("SignedHeaders=content-type;host;x-tc-action,"));

        let names: Vec<&str> = signed.headers().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "Authorization",
                "Content-Type",
                "Host",
                "X-TC-Action",
                "X-TC-Timestamp",
                "X-TC-Version",
                "X-TC-Region",
            ],
        );

        let unsigned: Vec<(&str, &str)> = signed
            .headers()
            .filter(|(name, _)| name.starts_with("X-TC-") && *name != "X-TC-Action")
            .collect();
        assert_eq!(
            unsigned,
            [
                ("X-TC-Timestamp", "1769904000"),
                ("X-TC-Version", "2023-09-01"),
                ("X-TC-Region", "ap-singapore"),
            ],
        );
    }

    #[test]
    fn the_action_header_is_sent_in_its_documented_case_and_signed_in_lower() {
        // Two rules pulling opposite ways: the canonical form lowercases header
        // values, and `X-TC-Action` is matched case-sensitively against the
        // action list. Sending the lowercased one is `InvalidAction`; signing
        // the cased one is `SignatureFailure`.
        let (call, credentials) = fixture();
        let signed = sign(&call, &credentials, FIXTURE_TIMESTAMP);
        let action = signed.headers().find(|(name, _)| *name == "X-TC-Action").unwrap().1;
        assert_eq!(action, "SubmitHunyuanTo3DProJob");

        // And the form the signature saw, taken from the canonical request the
        // same inputs build rather than inferred from the signature hex.
        let canonical = canonical_request(
            "POST",
            "/",
            "",
            &[("content-type", CONTENT_TYPE), ("host", call.host), ("x-tc-action", call.action)],
            call.body,
        );
        assert!(canonical.contains("\nx-tc-action:submithunyuanto3dprojob\n"), "{canonical}");
    }

    #[test]
    fn the_content_type_signed_is_the_one_the_adapter_is_told_to_send() {
        // The doc's own warning: a client that appends `; charset=utf-8` itself,
        // or drops it, sends a request that differs from the one signed. The
        // constant is exported so there is one string rather than two.
        let (call, credentials) = fixture();
        let signed = sign(&call, &credentials, FIXTURE_TIMESTAMP);
        let sent = signed.headers().find(|(name, _)| *name == "Content-Type").unwrap().1;
        assert_eq!(sent, CONTENT_TYPE);
        assert_eq!(sent, "application/json; charset=utf-8");
    }

    #[test]
    fn the_body_is_signed_exactly_as_given_so_re_serialising_it_is_visible() {
        // The hash covers the bytes, not the JSON. An adapter that signs a
        // `Value` and then re-serialises it before sending — reordering keys or
        // changing spacing — fails authentication, and this is the assertion
        // that says why.
        let (call, credentials) = fixture();
        let mut respaced = call;
        respaced.body = r#"{"Model": "3.1", "Prompt": "a mossy stone lantern"}"#;
        assert_ne!(
            sign(&call, &credentials, FIXTURE_TIMESTAMP).authorization(),
            sign(&respaced, &credentials, FIXTURE_TIMESTAMP).authorization(),
        );
    }

    #[test]
    fn the_timestamp_is_an_input_so_the_same_call_signs_differently_a_second_later() {
        // If this ever passes with equal signatures, something is reading the
        // clock internally and the vectors above have stopped testing anything.
        let (call, credentials) = fixture();
        assert_ne!(
            sign(&call, &credentials, FIXTURE_TIMESTAMP).authorization(),
            sign(&call, &credentials, FIXTURE_TIMESTAMP + 1).authorization(),
        );
    }

    #[test]
    fn the_region_rides_unsigned_so_two_regions_share_a_signature() {
        // Region is a common parameter and not part of the canonical request.
        // A signature that changed with it would mean the header set had
        // drifted, and the poll — which must target the submit's region — would
        // be the first thing to notice.
        let (call, credentials) = fixture();
        let mut elsewhere = call;
        elsewhere.region = "eu-frankfurt";
        assert_eq!(
            sign(&call, &credentials, FIXTURE_TIMESTAMP).authorization(),
            sign(&elsewhere, &credentials, FIXTURE_TIMESTAMP).authorization(),
        );
    }

    /* ── the secret ───────────────────────────────────────────────────────── */

    #[test]
    fn the_secret_key_never_appears_in_a_debug_dump_or_an_error() {
        // The equivalent of `keys.rs`'s
        // `a_key_never_appears_in_a_serialised_error_a_debug_dump_or_a_log_line`,
        // written just as wide: a credential escapes through whichever route
        // nobody checked, so every route out of this module is checked at once.
        // A Tencent `SecretKey` is an account-wide master credential, which
        // makes this the worst secret in the tree to lose.
        let key = SecretKey::new(FIXTURE_SECRET_KEY);
        assert_eq!(format!("{key:?}"), MASK);

        let credentials = Credentials::new(FIXTURE_SECRET_ID, key);
        let dumped = format!("{credentials:?}");
        assert!(!dumped.contains(FIXTURE_SECRET_KEY), "a key survived Debug: {dumped}");
        assert!(!dumped.contains(FIXTURE_SECRET_ID), "an account id survived Debug: {dumped}");
        assert!(dumped.contains("Credentials"), "the dump is still useful: {dumped}");

        // The signed output, which is the thing an adapter is most likely to
        // attach to a failed request.
        let (call, _) = fixture();
        let signed = sign(&call, &credentials, FIXTURE_TIMESTAMP);
        let dumped = format!("{signed:?}");
        assert!(!dumped.contains(FIXTURE_SECRET_KEY), "a key survived Debug: {dumped}");
        assert!(!dumped.contains(FIXTURE_SECRET_ID), "an account id survived Debug: {dumped}");
        assert!(!dumped.contains(FIXTURE_SIGNATURE), "a live signature survived Debug: {dumped}");
        assert!(dumped.contains("Authorization"), "the dump is still useful: {dumped}");

        // And every error this module can raise.
        for code in ["AuthFailure.SignatureExpire", "AuthFailure.SignatureFailure"] {
            let message = auth_failure(code).unwrap().to_string();
            assert!(!message.contains(FIXTURE_SECRET_KEY), "{message}");
            assert!(!message.contains(FIXTURE_SECRET_ID), "{message}");
        }
    }

    #[test]
    fn the_mask_is_the_only_string_a_formatter_can_get_out_of_a_secret_key() {
        // `Debug` is the only formatting trait `SecretKey` implements — there is
        // no `Display` and no `Serialize`, so `{}` and `serde_json::to_string`
        // do not compile against it, and this is the runtime half: whatever the
        // key contains, the impl is a constant and does not consult it.
        for key in ["", "short", FIXTURE_SECRET_KEY, "AKIDlookalike"] {
            assert_eq!(format!("{:?}", SecretKey::new(key)), MASK);
        }
    }

    /* ── the failures ─────────────────────────────────────────────────────── */

    #[test]
    fn an_expired_signature_tells_the_user_to_check_their_clock() {
        // The named requirement in `docs/08-providers.md`. Five minutes of drift
        // is something a desktop reaches on its own, and reporting it as "the
        // key was rejected" sends somebody to regenerate a credential that was
        // never the problem.
        let error = auth_failure("AuthFailure.SignatureExpire").unwrap();
        let message = error.to_string().to_lowercase();
        assert!(message.contains("clock"), "{message}");
        assert!(message.contains("system clock"), "{message}");
        assert!(!message.contains("api key"), "it must not read as a key problem: {message}");
        assert!(!error.is_retryable(), "retrying against the same wrong clock fails identically");
    }

    #[test]
    fn a_rejected_signature_that_is_not_expiry_is_reported_as_a_key_problem() {
        // The other half of the split. These two arrive as the same `AuthFailure`
        // prefix inside the same 200, so a `starts_with("AuthFailure")` would
        // collapse them.
        for code in [
            "AuthFailure.SignatureFailure",
            "AuthFailure.SecretIdNotFound",
            "AuthFailure.InvalidSecretId",
            "AuthFailure.TokenFailure",
            "AuthFailure.MFAFailure",
            "AuthFailure.UnauthorizedOperation",
        ] {
            let error = auth_failure(code).unwrap();
            assert_eq!(error.code(), "provider.bad_key", "{code}");
            assert!(error.to_string().contains(BACKEND), "{code}");
        }
    }

    #[test]
    fn an_error_outside_the_auth_family_is_left_for_the_adapter_to_answer() {
        // Signing has no opinion about an unactivated service or a full queue,
        // and a `None` here is what lets #64 map them without fighting this
        // file for the same string.
        assert!(auth_failure("FailedOperation.ServiceNotActivated").is_none());
        assert!(auth_failure("UnsupportedRegion").is_none());
        assert!(auth_failure("RequestLimitExceeded").is_none());
        assert!(auth_failure("").is_none());
    }
}
