//! Reading a streaming HTTP body: SSE framing, and doing it under a
//! cancellation.
//!
//! Shared by every adapter rather than written per vendor. That is not tidiness:
//! both vendors stream over the same `text/event-stream` framing and both are
//! cancelled the same way — by stopping holding the body — so a second copy of
//! this file would be a second place for the same subtle bug to be fixed in only
//! one of them. The adapters differ in what the payloads *mean*, which is what
//! their `wire.rs` is for.
//!
//! Split out from the adapters because this is the part of a streaming response
//! that a test can drive: the transport hands over bytes at whatever boundaries
//! it feels like, and every framing bug — a chunk that ends mid-line, a `\r\n`
//! from a proxy, a multi-byte character sawn in half — is invisible until it
//! happens to a user mid-generation.
//!
//! Only `data:` is read. The `event:` name is ignored on purpose: Anthropic
//! documents that every payload also carries the same name in its own `type`
//! field, and Google's Interactions API carries it as `event_type`, so decoding
//! one source rather than two removes the case where the two disagree and the
//! adapter has to pick.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use wobu_core::NodeKind;

use crate::cancel::Cancel;
use crate::error::Error;
use crate::provider::{DeltaSink, EnhanceOutcome};

/// Accumulates bytes and hands back complete `data:` payloads.
#[derive(Debug, Default)]
pub(crate) struct Sse {
    /// Bytes since the last newline. A chunk boundary lands here rather than in
    /// the middle of a decode, which is what makes a split multi-byte character
    /// a non-event: `\n` is ASCII, so anything up to one is whole UTF-8.
    partial: Vec<u8>,
    /// The `data:` lines of the event currently being assembled. SSE allows an
    /// event to span several of them; both vendors send one, but a decoder that
    /// only handles the shape it has seen is a decoder that breaks on a change
    /// nobody announced.
    data: String,
    ready: VecDeque<String>,
}

impl Sse {
    pub(crate) fn new() -> Sse {
        Sse::default()
    }

    /// Feed one chunk from the transport.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                let line = std::mem::take(&mut self.partial);
                let line = String::from_utf8_lossy(&line);
                self.line(line.trim_end_matches('\r'));
            } else {
                self.partial.push(byte);
            }
        }
    }

    /// The next complete payload, if one has arrived.
    pub(crate) fn next_event(&mut self) -> Option<String> {
        self.ready.pop_front()
    }

    fn line(&mut self, line: &str) {
        if line.is_empty() {
            // A blank line ends the event. An empty accumulator here is a
            // keep-alive gap, not an event with no data.
            if !self.data.is_empty() {
                self.ready.push_back(std::mem::take(&mut self.data));
            }
            return;
        }
        // Everything that is not a `data:` line falls through: `event:`, `id:`,
        // `retry:`, and `:`-prefixed comments, which some proxies inject as
        // keep-alives.
        if let Some(value) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
}

/// What one turn of an adapter's read loop got.
pub(crate) enum Read<B, E> {
    Chunk(std::result::Result<B, E>),
    End,
    Cancelled,
}

/// The next chunk, or the cancellation that beat it.
///
/// Cancellation is polled first so that a token set while a chunk was already
/// waiting still wins: the point is to stop, and a stream with a chunk ready is
/// a stream that will have another one ready in a moment.
pub(crate) async fn next_chunk<S, B, E>(mut body: Pin<&mut S>, cancel: &Cancel) -> Read<B, E>
where
    S: Stream<Item = std::result::Result<B, E>>,
{
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Read::Cancelled);
        }
        match body.as_mut().poll_next(cx) {
            Poll::Ready(Some(chunk)) => Poll::Ready(Read::Chunk(chunk)),
            Poll::Ready(None) => Poll::Ready(Read::End),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

/// `None` when the cancellation won, in which case `future` is dropped — which
/// for an in-flight request is what abandons it.
pub(crate) async fn until_cancelled<F: Future>(future: F, cancel: &Cancel) -> Option<F::Output> {
    let mut future = std::pin::pin!(future);
    let mut cancelled = std::pin::pin!(cancel.cancelled());
    std::future::poll_fn(move |cx: &mut Context<'_>| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        future.as_mut().poll(cx).map(Some)
    })
    .await
}

/// The provider-owned state machine consumed by the shared SSE lifecycle.
/// Implementations decide what an event means and how accumulated bytes, usage,
/// and provider status become an Enhance outcome.
pub(crate) trait SseConsumer {
    fn event(&mut self, payload: &str, deltas: &mut dyn DeltaSink) -> bool;
    fn finish(self, kind: NodeKind, aborted: Option<Error>) -> EnhanceOutcome;
}

/// Read an SSE response until its provider state machine is done, the socket
/// ends, transport fails, or cancellation wins.
pub(crate) async fn read_sse<S, B, E, C>(
    kind: NodeKind,
    body: S,
    deltas: &mut dyn DeltaSink,
    cancel: &Cancel,
    mut consumer: C,
) -> EnhanceOutcome
where
    S: Stream<Item = std::result::Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::fmt::Display,
    C: SseConsumer,
{
    let mut body = std::pin::pin!(body);
    let mut sse = Sse::new();

    loop {
        match next_chunk(body.as_mut(), cancel).await {
            Read::Chunk(Ok(bytes)) => {
                sse.push(bytes.as_ref());
                while let Some(event) = sse.next_event() {
                    if consumer.event(&event, deltas) {
                        return consumer.finish(kind, None);
                    }
                }
            }
            Read::Chunk(Err(error)) => {
                return consumer
                    .finish(kind, Some(Error::Unavailable { detail: error.to_string() }));
            }
            Read::End => return consumer.finish(kind, None),
            Read::Cancelled => return consumer.finish(kind, Some(Error::Cancelled)),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    use futures_core::Stream;
    use serde_json::json;
    use wobu_core::NodeKind;
    use wobu_core::schema::description_schema;

    use super::{SseConsumer, read_sse};
    use crate::cancel::Cancel;
    use crate::error::Error;
    use crate::provider::Discard;

    pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
        struct Unparker(std::thread::Thread);
        impl Wake for Unparker {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }

        let waker = Waker::from(Arc::new(Unparker(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }

    pub(crate) struct Body {
        pub(crate) chunks: VecDeque<std::result::Result<Vec<u8>, String>>,
        pub(crate) polls: Arc<AtomicUsize>,
        quiet: bool,
    }

    impl Body {
        pub(crate) fn new(chunks: Vec<std::result::Result<Vec<u8>, String>>) -> Body {
            Body { chunks: chunks.into(), polls: Arc::new(AtomicUsize::new(0)), quiet: false }
        }

        pub(crate) fn quiet_after(mut self) -> Body {
            self.quiet = true;
            self
        }
    }

    impl Stream for Body {
        type Item = std::result::Result<Vec<u8>, String>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.get_mut();
            this.polls.fetch_add(1, Ordering::SeqCst);
            match this.chunks.pop_front() {
                Some(chunk) => Poll::Ready(Some(chunk)),
                None if this.quiet => Poll::Pending,
                None => Poll::Ready(None),
            }
        }
    }

    /// A description built from the registry schema, shared by each adapter's
    /// contract suite so their wire fixtures exercise identical application
    /// data.
    pub(crate) fn document(kind: NodeKind) -> String {
        let schema = description_schema(kind);
        let mut object = serde_json::Map::new();
        for (key, property) in schema["properties"].as_object().unwrap() {
            let value = match property["type"].as_str().unwrap() {
                "string" => json!("A scout in ash-glazed plate, shoulders canted forward."),
                "array" if property["items"]["pattern"].is_string() => {
                    json!(["#2b2118", "#c2703a"])
                }
                _ => json!(["Ember-lit throat vents"]),
            };
            object.insert(key.clone(), value);
        }
        serde_json::to_string(&serde_json::Value::Object(object)).unwrap()
    }

    /// Split independently of SSE frame boundaries, as a socket is free to do.
    pub(crate) fn split(bytes: &[u8], size: usize) -> Vec<Result<Vec<u8>, String>> {
        bytes.chunks(size).map(|chunk| Ok(chunk.to_vec())).collect()
    }

    pub(crate) fn assert_complete<C: SseConsumer>(bytes: Vec<u8>, consumer: C) {
        let expected = document(NodeKind::Character);
        let mut streamed = String::new();
        let mut sink = |json: &str| streamed.push_str(json);
        let outcome = block_on(read_sse(
            NodeKind::Character,
            Body::new(split(&bytes, 37)),
            &mut sink,
            &Cancel::new(),
            consumer,
        ));

        let validated = outcome.result.expect("a whole provider response should validate");
        assert_eq!(streamed, expected);
        assert_eq!(crate::parse_description(NodeKind::Character, &streamed).unwrap(), validated);
        assert_eq!(outcome.usage.input_tokens, 812);
        assert_eq!(outcome.usage.output_tokens, 289);
    }

    pub(crate) fn assert_every_kind<C, F, W>(provider: &str, consumer: F, wire: W)
    where
        C: SseConsumer,
        F: Fn() -> C,
        W: Fn(&str) -> Vec<u8>,
    {
        for def in wobu_core::kind::kind_registry() {
            let outcome = block_on(read_sse(
                def.kind,
                Body::new(split(&wire(&document(def.kind)), 64)),
                &mut Discard,
                &Cancel::new(),
                consumer(),
            ));
            outcome.result.unwrap_or_else(|error| {
                panic!(
                    "{} could not round-trip its own schema through {provider}: {error}",
                    def.kind
                )
            });
        }
    }

    pub(crate) fn assert_billed_disconnect<C: SseConsumer>(bytes: &[u8], consumer: C) {
        let mut chunks = split(&bytes[..bytes.len() / 2], 37);
        chunks.push(Err("connection reset by peer".to_string()));
        let outcome = block_on(read_sse(
            NodeKind::Character,
            Body::new(chunks),
            &mut Discard,
            &Cancel::new(),
            consumer,
        ));

        assert!(matches!(outcome.result, Err(Error::Unavailable { .. })));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    pub(crate) fn assert_truncated<C: SseConsumer>(bytes: &[u8], consumer: C) {
        let outcome = block_on(read_sse(
            NodeKind::Character,
            Body::new(split(bytes, 37)),
            &mut Discard,
            &Cancel::new(),
            consumer,
        ));
        assert!(matches!(outcome.result, Err(Error::Truncated)));
    }

    pub(crate) fn assert_mid_stream_cancel<C: SseConsumer>(bytes: &[u8], consumer: C) {
        let body = Body::new(split(bytes, 24));
        let polls = Arc::clone(&body.polls);
        let total = body.chunks.len();
        let cancel = Cancel::new();
        let mut seen = 0usize;
        let mut sink = |_: &str| {
            seen += 1;
            if seen == 2 {
                cancel.cancel();
            }
        };
        let outcome = block_on(read_sse(NodeKind::Character, body, &mut sink, &cancel, consumer));

        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert!(
            polls.load(Ordering::SeqCst) < total,
            "the loop read {} of {total} chunks after Stop",
            polls.load(Ordering::SeqCst),
        );
        assert_eq!(outcome.usage.input_tokens, 812, "the prompt was billed before Stop");
    }

    pub(crate) fn assert_quiet_cancel<C: SseConsumer>(billed: &[u8], consumer: C) {
        let body = Body::new(split(billed, 64)).quiet_after();
        let cancel = Cancel::new();
        let stop = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            stop.cancel();
        });
        let outcome =
            block_on(read_sse(NodeKind::Character, body, &mut Discard, &cancel, consumer));

        assert!(matches!(outcome.result, Err(Error::Cancelled)));
        assert_eq!(outcome.usage.input_tokens, 812);
    }

    pub(crate) fn assert_pre_cancel<C: SseConsumer>(bytes: &[u8], consumer: C) {
        let cancel = Cancel::new();
        cancel.cancel();
        let outcome = block_on(read_sse(
            NodeKind::Character,
            Body::new(split(bytes, 24)),
            &mut Discard,
            &cancel,
            consumer,
        ));
        let error = outcome.result.expect_err("a cancelled call has no description");
        assert!(!error.is_retryable());
        assert_eq!(error.code(), "cancelled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(chunks: &[&str]) -> Vec<String> {
        let mut sse = Sse::new();
        let mut seen = Vec::new();
        for chunk in chunks {
            sse.push(chunk.as_bytes());
            while let Some(event) = sse.next_event() {
                seen.push(event);
            }
        }
        seen
    }

    #[test]
    fn a_payload_arrives_once_its_blank_line_does() {
        let seen = events(&["event: ping\ndata: {\"type\":\"ping\"}\n\n"]);
        assert_eq!(seen, ["{\"type\":\"ping\"}"]);
    }

    #[test]
    fn a_payload_split_across_chunks_is_reassembled() {
        // The regression: a decoder that treats each chunk as a frame. Nothing
        // in HTTP promises a chunk is a line, and the failure looks like a
        // provider sending malformed JSON.
        let seen =
            events(&["event: mess", "age_stop\nda", "ta: {\"type\":\"mes", "sage_stop\"}\n", "\n"]);
        assert_eq!(seen, ["{\"type\":\"message_stop\"}"]);
    }

    #[test]
    fn a_chunk_boundary_inside_a_multi_byte_character_does_not_corrupt_it() {
        // Descriptions are full of accented names, and a chunk can split any
        // byte. Decoding per chunk would put a replacement character into a
        // node's canon.
        let payload = "data: {\"text\":\"Vashk of Kön\"}\n\n".as_bytes();
        let split = payload.iter().position(|&b| b == 0xc3).unwrap() + 1;
        let mut sse = Sse::new();
        sse.push(&payload[..split]);
        sse.push(&payload[split..]);
        assert_eq!(sse.next_event().unwrap(), "{\"text\":\"Vashk of Kön\"}");
    }

    #[test]
    fn carriage_returns_from_a_proxy_are_not_part_of_the_payload() {
        // A `\r` left on the end makes every payload fail to parse as JSON, and
        // only on the networks that have such a proxy in the way.
        let seen = events(&["data: {\"type\":\"ping\"}\r\n\r\n"]);
        assert_eq!(seen, ["{\"type\":\"ping\"}"]);
    }

    #[test]
    fn a_payload_may_be_written_with_or_without_the_space_after_the_colon() {
        let seen = events(&["data:{\"a\":1}\n\n", "data:  {\"b\":2}\n\n"]);
        assert_eq!(seen, ["{\"a\":1}", " {\"b\":2}"]);
    }

    #[test]
    fn several_data_lines_join_with_newlines_as_the_format_says() {
        let seen = events(&["data: {\n", "data: \"a\": 1}\n", "\n"]);
        assert_eq!(seen, ["{\n\"a\": 1}"]);
    }

    #[test]
    fn comments_and_unknown_fields_are_not_payloads() {
        // Keep-alive comments are how some proxies hold the connection open.
        // Treating one as an event would hand the accumulator garbage.
        let seen = events(&[": keep-alive\n\nid: 7\nretry: 500\ndata: {\"a\":1}\n\n"]);
        assert_eq!(seen, ["{\"a\":1}"]);
    }

    #[test]
    fn an_event_cut_off_by_a_dead_connection_is_never_handed_over() {
        // A half-arrived payload is not a payload. Emitting it would put
        // truncated JSON in front of the accumulator, which would report a bad
        // response rather than the dropped connection that actually happened.
        let seen = events(&["data: {\"type\":\"message_"]);
        assert!(seen.is_empty());
    }

    #[test]
    fn a_sentinel_payload_that_is_not_json_still_frames_cleanly() {
        // Google closes an Interactions stream with `event: done` /
        // `data: [DONE]`. The framing has to hand that over like anything else
        // so the adapter — not the decoder — decides it means nothing.
        let seen = events(&["event: done\ndata: [DONE]\n\n"]);
        assert_eq!(seen, ["[DONE]"]);
    }
}
