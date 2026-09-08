//! Provider-neutral HTTP mechanics shared by text and image adapters.
//!
//! This module deliberately stops at bytes and transport failures. Provider
//! payloads, authentication headers, status mappings, billing decisions, and
//! user-facing wording stay in their adapters.

use std::time::Duration;

use reqwest::{RequestBuilder, Response};
use serde::Serialize;
use serde_json::Value;

use crate::provider::{DeltaSink, EnhanceOutcome, EnhanceRequest, StructuredRequest};
use crate::stream::{SseConsumer, read_sse};
use crate::{Cancel, Error};

/// Every remote provider uses the same connection timeout. There is no whole
/// request timeout: generation may legitimately be slow and is cancelled by
/// the job token instead.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A failure before an adapter has a response it can interpret.
#[derive(Debug)]
pub enum Failure {
    Cancelled,
    Unavailable(reqwest::Error),
}

/// Build the common pooled HTTP client without choosing an adapter error type.
pub fn client() -> std::result::Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())
}

/// Send a fully provider-owned request, abandoning it immediately when the
/// cancellation wins.
pub async fn send(request: RequestBuilder, cancel: &Cancel) -> Result<Response, Failure> {
    match crate::stream::until_cancelled(request.send(), cancel).await {
        None => Err(Failure::Cancelled),
        Some(Err(error)) => Err(Failure::Unavailable(error)),
        Some(Ok(response)) => Ok(response),
    }
}

/// The provider's explicit backoff, if its response carries one.
pub fn retry_after(response: &Response) -> Option<Duration> {
    response.headers().get("retry-after").and_then(retry_after_value)
}

/// A request-free status detail suitable for provider error logs. Keeping this
/// mechanical formatting here lets adapters choose their own error variants
/// and wording without duplicating the redaction boundary.
pub fn status_detail(status: u16, code: &str, message: &str) -> String {
    match (code.is_empty(), message.is_empty()) {
        (true, true) => format!("HTTP {status}"),
        (true, false) => format!("HTTP {status}: {message}"),
        (false, _) => format!("HTTP {status} {code}: {message}"),
    }
}

fn retry_after_value(value: &reqwest::header::HeaderValue) -> Option<Duration> {
    value.to_str().ok().and_then(|value| value.trim().parse::<u64>().ok()).map(Duration::from_secs)
}

/// Read a response body under cancellation. A transport error after a status
/// has arrived becomes an empty body so the adapter can still map that status,
/// preserving the existing behavior of all three remote adapters.
pub async fn bytes_or_empty(response: Response, cancel: &Cancel) -> Result<Vec<u8>, Failure> {
    match crate::stream::until_cancelled(response.bytes(), cancel).await {
        None => Err(Failure::Cancelled),
        Some(body) => Ok(body.map(|bytes| bytes.to_vec()).unwrap_or_default()),
    }
}

/// The text equivalent of [`bytes_or_empty`]. `reqwest` retains responsibility
/// for decoding the response exactly as it did in the original adapters.
pub async fn text_or_empty(response: Response, cancel: &Cancel) -> Result<String, Failure> {
    match crate::stream::until_cancelled(response.text(), cancel).await {
        None => Err(Failure::Cancelled),
        Some(body) => Ok(body.unwrap_or_default()),
    }
}

/// Serialize one provider-owned payload into request bytes while preserving the
/// text adapters' public schema error.
pub(crate) fn json_body<T: Serialize>(value: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|error| Error::SchemaRejected { detail: error.to_string() })
}

/// Common text-adapter request lifecycle. The callback remains provider-owned:
/// it receives the exact status, body, and retry hint and returns that adapter's
/// established public error.
pub(crate) async fn text_stream<F>(
    request: RequestBuilder,
    cancel: &Cancel,
    status_error: F,
) -> Result<Response, Error>
where
    F: FnOnce(u16, &str, Option<Duration>) -> Error,
{
    let response = send(request, cancel).await.map_err(text_failure)?;
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status().as_u16();
    let retry_after = retry_after(&response);
    let body = text_or_empty(response, cancel).await.map_err(text_failure)?;
    Err(status_error(status, &body, retry_after))
}

/// The provider-shaped half of a streaming enhance.
///
/// Everything an adapter has to decide for itself and nothing it does not:
/// where to send, how to authenticate, what the body looks like, how a status
/// maps to an error, and what reads the events back.
pub(crate) trait SseEnhance {
    type Consumer: SseConsumer;

    fn client(&self) -> &reqwest::Client;

    fn base_url(&self) -> &str;

    /// Authentication, and any header only this provider's API asks for.
    /// `content-type` and `accept` are added by [`enhance_over_sse`].
    fn authenticate(&self, request: RequestBuilder) -> RequestBuilder;

    fn request_body(request: &EnhanceRequest) -> Value;

    fn structured_body(request: &StructuredRequest) -> Value;

    fn structured_consumer() -> Self::Consumer;

    fn error_for_status(status: u16, body: &str, retry_after: Option<Duration>) -> Error;

    fn consumer() -> Self::Consumer;
}

/// The half of `TextProvider::enhance` that is the same for every adapter.
///
/// The pre-cancel check, serialising the body, opening the stream and handing
/// it to the SSE reader are provider-neutral, and the two adapters that had a
/// copy each differed only in their authentication headers. One copy means the
/// rule that an unbilled failure stays unbilled is stated once.
pub(crate) async fn enhance_over_sse<P: SseEnhance>(
    provider: &P,
    request: &EnhanceRequest,
    deltas: &mut dyn DeltaSink,
    cancel: &Cancel,
) -> EnhanceOutcome {
    // A job cancelled while it was queued should not open a connection at all.
    // Everything before the first byte of response is unbilled.
    if cancel.is_cancelled() {
        return EnhanceOutcome::unbilled(Error::Cancelled);
    }

    let body = match json_body(&P::request_body(request)) {
        Ok(body) => body,
        // Only reachable if the generated schema is not serialisable, which is
        // our bug in the same way a rejected schema is.
        Err(error) => return EnhanceOutcome::unbilled(error),
    };

    let send = provider
        .authenticate(provider.client().post(provider.base_url()))
        .header("content-type", "application/json")
        // Streaming is asked for in the body; this only stops a proxy deciding
        // to buffer the response into one lump.
        .header("accept", "text/event-stream")
        .body(body);

    let response = match text_stream(send, cancel, P::error_for_status).await {
        Ok(response) => response,
        Err(error) => return EnhanceOutcome::unbilled(error),
    };

    read_sse(request.kind, response.bytes_stream(), deltas, cancel, P::consumer()).await
}

fn text_failure(failure: Failure) -> Error {
    match failure {
        Failure::Cancelled => Error::Cancelled,
        Failure::Unavailable(error) => Error::Unavailable { detail: error.to_string() },
    }
}

/// Both text adapters share this delegation, while the authentication expression
/// and each wire module remain vendor-owned. Extending the authoring capability
/// must not leave one adapter on a different request lifecycle.
macro_rules! text_adapter {
    ($provider:ty, $id:expr, $label:expr, $model:expr, $wire:ident, $authenticate:expr) => {
        #[async_trait::async_trait]
        impl $crate::TextProvider for $provider {
            fn id(&self) -> &'static str {
                $id
            }
            fn label(&self) -> &'static str {
                $label
            }
            fn default_model(&self) -> &'static str {
                $model
            }
            fn supports_structured(&self) -> bool {
                true
            }
            async fn structured(
                &self,
                request: &$crate::StructuredRequest,
                deltas: &mut dyn $crate::DeltaSink,
                cancel: &$crate::Cancel,
            ) -> $crate::StructuredOutcome {
                $crate::structured::generate(self, request, deltas, cancel).await
            }
            async fn enhance(
                &self,
                request: &$crate::EnhanceRequest,
                deltas: &mut dyn $crate::DeltaSink,
                cancel: &$crate::Cancel,
            ) -> $crate::EnhanceOutcome {
                $crate::transport::enhance_over_sse(self, request, deltas, cancel).await
            }
        }
        impl $crate::transport::SseEnhance for $provider {
            type Consumer = $wire::Incoming;
            fn client(&self) -> &reqwest::Client {
                &self.client
            }
            fn base_url(&self) -> &str {
                &self.base_url
            }
            fn authenticate(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
                ($authenticate)(self, request)
            }
            fn request_body(request: &$crate::EnhanceRequest) -> serde_json::Value {
                $wire::request_body(request)
            }
            fn structured_body(request: &$crate::StructuredRequest) -> serde_json::Value {
                $wire::structured_body(request)
            }
            fn error_for_status(
                status: u16,
                body: &str,
                retry_after: Option<std::time::Duration>,
            ) -> $crate::Error {
                $wire::error_for_status(status, body, retry_after)
            }
            fn consumer() -> Self::Consumer {
                $wire::Incoming::new()
            }
            fn structured_consumer() -> Self::Consumer {
                $wire::Incoming::structured()
            }
        }
    };
}
pub(crate) use text_adapter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_seconds_and_rejects_everything_else() {
        let seconds = reqwest::header::HeaderValue::from_static("42");
        assert_eq!(retry_after_value(&seconds), Some(Duration::from_secs(42)));

        let words = reqwest::header::HeaderValue::from_static("later");
        assert_eq!(retry_after_value(&words), None);
    }
}
