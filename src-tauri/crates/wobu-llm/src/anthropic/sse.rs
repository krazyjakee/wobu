//! Turning a stream of arbitrary byte chunks into whole SSE payloads.
//!
//! Split out from the adapter because this is the part of a streaming response
//! that a test can drive: the transport hands over bytes at whatever boundaries
//! it feels like, and every framing bug — a chunk that ends mid-line, a `\r\n`
//! from a proxy, a multi-byte character sawn in half — is invisible until it
//! happens to a user mid-generation.
//!
//! Only `data:` is read. The `event:` name is ignored on purpose: Anthropic
//! documents that every payload also carries the same name in its own `type`
//! field, so decoding one source rather than two removes the case where the two
//! disagree and the adapter has to pick.

use std::collections::VecDeque;

/// Accumulates bytes and hands back complete `data:` payloads.
#[derive(Debug, Default)]
pub(crate) struct Sse {
    /// Bytes since the last newline. A chunk boundary lands here rather than in
    /// the middle of a decode, which is what makes a split multi-byte character
    /// a non-event: `\n` is ASCII, so anything up to one is whole UTF-8.
    partial: Vec<u8>,
    /// The `data:` lines of the event currently being assembled. SSE allows an
    /// event to span several of them; Anthropic sends one, but a decoder that
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
        let seen = events(&["event: mess", "age_stop\nda", "ta: {\"type\":\"mes", "sage_stop\"}\n", "\n"]);
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
}
