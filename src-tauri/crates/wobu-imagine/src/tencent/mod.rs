//! Tencent Hunyuan3D: the signing half.
//!
//! The mesh backend, and the only provider in the tree with no bearer key. A
//! `SecretId`/`SecretKey` pair is signed per request with TC3-HMAC-SHA256, so
//! there is no "paste a key into a header" path at all — the credential is used
//! to *derive* something the request carries, and every mistake in that
//! derivation reports itself as the same auth failure.
//!
//! That is why this module exists ahead of the adapter
//! ([#64](https://github.com/krazyjakee/wobu/issues/64)) rather than inside it:
//! `sign.rs` is a pure function of strings and a timestamp, which is the only
//! shape that can be checked against Tencent's published vectors instead of
//! against a live account. The HTTP, the job submit/poll loop, the region
//! handling and the rest of the error surface are #64's.
//!
//! See the `Tencent Hunyuan3D` section of `docs/08-providers.md` for what the
//! signed header set is and why, and `sign.rs`'s own note for where the test
//! vectors came from and which of them are Tencent's.

mod sign;

pub use sign::{CONTENT_TYPE, Call, Credentials, SecretKey, Signed, auth_failure, sign};
