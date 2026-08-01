//! Reading a half-arrived document, so the editor can type it out.
//!
//! Both adapters stream the *structured document itself* — Anthropic as
//! `input_json_delta` on the tool block, Gemini as partial text of the JSON body
//! — so there is no point in the stream where a whole section exists as a
//! string. [`DeltaSink`](crate::DeltaSink) says as much, and says where the
//! reader that fixes it belongs: above the trait, in this crate, rather than
//! copied into each adapter where the same subtle bug would be fixed in only one
//! of them.
//!
//! **Nothing here decides anything.** What this produces is display state on its
//! way to `enhance:delta`; the only route to a description a node will hold is
//! [`parse_description`](crate::parse_description), run on the whole text after
//! the call ended. That split is what the rule "never write a partial stream to
//! a node" is made of, and it is why this reader is allowed to be lenient: a
//! fragment it reads wrongly costs a repaint, never a write.

use wobu_core::SectionValue;

/// Every section that has begun arriving, in the order the document writes them,
/// carrying whatever has landed so far.
///
/// [`SectionValue`] rather than a partial-shaped type of its own, because the
/// editor already renders one of those and a second shape would mean a second
/// renderer. A text section that has opened and streamed nothing is
/// `Text("")` — present, so its heading can appear, and empty, so nothing draws
/// under it.
pub fn read_partial(json: &str) -> Vec<(String, SectionValue)> {
    // Collected rather than scanned as bytes because escapes are decoded here
    // and `😀` is two escapes for one character. A response is a few
    // kilobytes at the very most, so the allocation is not worth avoiding.
    let chars: Vec<char> = json.chars().collect();
    let mut scan = Scan { chars: &chars, at: 0 };
    let mut out: Vec<(String, SectionValue)> = Vec::new();

    scan.skip_ws();
    if !scan.eat('{') {
        return out;
    }
    loop {
        scan.skip_ws();
        match scan.peek() {
            Some(',') => {
                scan.at += 1;
                continue;
            }
            Some('"') => {}
            // A closing brace, the end of what has arrived, or something that is
            // not a document at all. All three mean "nothing more to draw".
            _ => return out,
        }
        // A key that is still arriving names nothing yet, so there is no row to
        // put anything in.
        let Some((key, true)) = scan.string() else { return out };
        scan.skip_ws();
        if !scan.eat(':') {
            return out;
        }
        scan.skip_ws();
        match scan.peek() {
            Some('"') => {
                let Some((text, _)) = scan.string() else { return out };
                out.push((key, SectionValue::Text(text)));
            }
            Some('[') => out.push((key, SectionValue::List(scan.strings()))),
            // A section of some other type is a response the validator is going
            // to reject anyway. Stopping rather than skipping past it, because
            // nothing after a value this reader did not understand can be
            // trusted to line up with a key.
            _ => return out,
        }
    }
}

struct Scan<'a> {
    chars: &'a [char],
    at: usize,
}

impl Scan<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let found = self.peek();
        if found.is_some() {
            self.at += 1;
        }
        found
    }

    fn eat(&mut self, want: char) -> bool {
        let found = self.peek() == Some(want);
        if found {
            self.at += 1;
        }
        found
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    /// A JSON string from the cursor, and whether its closing quote arrived.
    ///
    /// The flag is what tells a finished key from one still being typed. For a
    /// *value* it does not matter — half a sentence is exactly what the editor
    /// is meant to show — which is why callers ignore it there.
    fn string(&mut self) -> Option<(String, bool)> {
        if !self.eat('"') {
            return None;
        }
        let mut out = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => return Some((out, true)),
                '\\' => match self.escape() {
                    Some(decoded) => out.push(decoded),
                    // The escape is still in flight. Stopping short of it keeps
                    // a half-written `é` from flickering as a stray glyph
                    // for one frame before it becomes an é.
                    None => return Some((out, false)),
                },
                _ => out.push(c),
            }
        }
        Some((out, false))
    }

    fn escape(&mut self) -> Option<char> {
        match self.bump()? {
            '"' => Some('"'),
            '\\' => Some('\\'),
            '/' => Some('/'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'u' => self.unicode_escape(),
            // Not an escape JSON defines, so the document is malformed rather
            // than incomplete. Saying so is `validate.rs`'s job on the finished
            // text; here it just stops the section growing.
            _ => None,
        }
    }

    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex4()?;
        if !(0xD800..0xDC00).contains(&first) {
            return char::from_u32(first);
        }
        // A lone surrogate is half a character and the other half may still be
        // arriving, so the cursor is put back and the section waits for it. An
        // emoji appearing as a replacement glyph and then correcting itself is
        // the exact flicker this reader exists to avoid.
        let mark = self.at;
        if self.eat('\\')
            && self.eat('u')
            && let Some(low) = self.hex4()
            && (0xDC00..0xE000).contains(&low)
        {
            return char::from_u32(0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00));
        }
        self.at = mark;
        None
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            value = value * 16 + self.bump()?.to_digit(16)?;
        }
        Some(value)
    }

    /// An array of strings, including the one still being written.
    fn strings(&mut self) -> Vec<String> {
        let mut items = Vec::new();
        if !self.eat('[') {
            return items;
        }
        loop {
            self.skip_ws();
            match self.peek() {
                Some(']') => {
                    self.at += 1;
                    return items;
                }
                Some(',') => self.at += 1,
                Some('"') => match self.string() {
                    Some((item, _)) => items.push(item),
                    None => return items,
                },
                // The end of what has arrived, or a non-string item the
                // validator will reject on the way out.
                _ => return items,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::NodeKind;

    fn text(sections: &[(String, SectionValue)], key: &str) -> Option<String> {
        sections.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            SectionValue::Text(t) => Some(t.clone()),
            SectionValue::List(_) => None,
        })
    }

    fn list(sections: &[(String, SectionValue)], key: &str) -> Option<Vec<String>> {
        sections.iter().find(|(k, _)| k == key).and_then(|(_, v)| match v {
            SectionValue::List(items) => Some(items.clone()),
            SectionValue::Text(_) => None,
        })
    }

    #[test]
    fn a_section_shows_as_far_as_it_has_arrived_rather_than_waiting_for_its_quote() {
        // The whole point of the reader. Waiting for a closing quote means the
        // pane sits blank through the one part of an Enhance the user is
        // watching, and then fills in all at once.
        let sections = read_partial(r#"{"silhouette": "Tall, narrow-should"#);
        assert_eq!(text(&sections, "silhouette").as_deref(), Some("Tall, narrow-should"));
    }

    #[test]
    fn every_prefix_of_a_document_reads_as_a_prefix_of_the_last_one() {
        // The property the typing effect is: text only ever grows, and a section
        // never appears, disappears and comes back. A reader that returned less
        // than it had a fragment earlier would show the pane deleting itself.
        let document = r##"{"silhouette":"Tall, narrow, hooded","never":["neon","chrome"],
                            "palette":["#2b2118"]}"##;
        let chars: Vec<char> = document.chars().collect();

        let mut previous: Vec<(String, SectionValue)> = Vec::new();
        for end in 0..=chars.len() {
            let so_far: String = chars[..end].iter().collect();
            let now = read_partial(&so_far);
            assert!(now.len() >= previous.len(), "a section vanished at {end}: {so_far}");
            for (before, after) in previous.iter().zip(&now) {
                assert_eq!(before.0, after.0, "the sections reordered at {end}");
                match (&before.1, &after.1) {
                    (SectionValue::Text(was), SectionValue::Text(is)) => {
                        assert!(is.starts_with(was.as_str()), "{was:?} was rewritten as {is:?}");
                    }
                    (SectionValue::List(was), SectionValue::List(is)) => {
                        assert!(is.len() >= was.len(), "a list item vanished at {end}");
                    }
                    _ => panic!("a section changed type at {end}"),
                }
            }
            previous = now;
        }

        // And the end state is the whole document.
        assert_eq!(text(&previous, "silhouette").as_deref(), Some("Tall, narrow, hooded"));
        assert_eq!(list(&previous, "never"), Some(vec!["neon".into(), "chrome".into()]));
    }

    #[test]
    fn a_list_shows_the_item_that_is_still_being_written() {
        let sections = read_partial(r#"{"never":["modern firearms","ne"#);
        assert_eq!(list(&sections, "never"), Some(vec!["modern firearms".into(), "ne".into()]));
    }

    #[test]
    fn nothing_is_read_before_the_document_has_opened() {
        // Anthropic sends a `content_block_start` with an empty input before the
        // first `input_json_delta`, so this state is on the wire every time.
        assert!(read_partial("").is_empty());
        assert!(read_partial("  ").is_empty());
        assert!(read_partial("{").is_empty());
        assert!(read_partial(r#"{"silh"#).is_empty(), "a half-typed key names nothing");
    }

    #[test]
    fn escapes_are_decoded_and_a_half_arrived_one_is_held_back() {
        // The flicker this guards: `\n` shown as a backslash and an n for one
        // frame, and `é` shown as four hex digits before it becomes an é.
        let whole = read_partial(r#"{"anatomy":"Four-jointed.\nAsh-glazed élan."}"#);
        assert_eq!(
            text(&whole, "anatomy").as_deref(),
            Some("Four-jointed.\nAsh-glazed élan."),
        );

        for cut in [r#"{"anatomy":"Four-jointed.\"#, r#"{"anatomy":"Four-jointed.\u00"#] {
            assert_eq!(text(&read_partial(cut), "anatomy").as_deref(), Some("Four-jointed."));
        }
    }

    #[test]
    fn a_surrogate_pair_waits_for_its_second_half_rather_than_drawing_a_placeholder() {
        let half = read_partial(r#"{"signature":["Ember \ud83d"#);
        assert_eq!(list(&half, "signature"), Some(vec!["Ember ".into()]));

        let whole = read_partial(r#"{"signature":["Ember 🔥"]}"#);
        assert_eq!(list(&whole, "signature"), Some(vec!["Ember 🔥".into()]));
    }

    #[test]
    fn what_the_pane_showed_last_is_what_the_validator_then_accepts() {
        // The regression worth having: the editor typing out one description
        // while the node ends up holding another. Both sides read the same
        // accumulated text, and this is the assertion that they agree about it.
        // Built from the registry rather than spelled out, so a kind that gains
        // a section is covered without anyone remembering to come here.
        let mut object = serde_json::Map::new();
        for def in wobu_core::kind::kind_def(NodeKind::Character).sections {
            let value = match (def.value_kind, def.key) {
                (wobu_core::kind::SectionValueKind::Text, _) => {
                    serde_json::json!("Tall, narrow-shouldered, forward-canted")
                }
                (_, "palette") => serde_json::json!(["#2b2118", "#c2703a"]),
                _ => serde_json::json!(["Ember-lit throat vents"]),
            };
            object.insert(def.key.to_string(), value);
        }
        let document = serde_json::Value::Object(object).to_string();

        let shown = read_partial(&document);
        let validated = crate::parse_description(NodeKind::Character, &document)
            .expect("the document satisfies the schema");
        for (key, value) in &validated.description.sections {
            assert_eq!(
                shown.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                Some(value),
                "the pane and the node disagree about `{key}`",
            );
        }
    }
}
