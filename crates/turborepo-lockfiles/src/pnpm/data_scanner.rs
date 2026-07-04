//! Structural line scanner for the pnpm lockfile YAML subset.
//!
//! pnpm-lock.yaml is machine-generated, line-oriented YAML: block mappings
//! indented with spaces, plain or single-quoted scalars, one-line flow
//! collections for `resolution:`/`engines:`/`os:`, block sequences of
//! scalars, and the occasional folded (`>-`) scalar for `deprecated:`
//! messages. A general YAML scanner spends most of lockfile parsing in its
//! state machine; this scanner instead walks the input line by line with
//! `memchr`, emitting the same [`saphyr_parser::Event`] stream the semantic
//! layer in `data_fast_parse` already consumes.
//!
//! Correctness strategy is inherited from the fast-parse tier: anything
//! outside the shapes this scanner fully understands returns
//! [`Unsupported`], and the caller falls back to the saphyr event parser
//! (and from there to serde). The scanner must never *accept* input with a
//! different meaning than saphyr would assign — differential tests enforce
//! scanner == saphyr == serde on everything the scanner accepts. Notably,
//! anything saphyr would *reject* (e.g. `a: b: c`) must be rejected here
//! too, since accepting it would fabricate a lockfile where the serde path
//! reports an error.

use std::borrow::Cow;

use saphyr_parser::{Event, ScalarStyle};

use super::Unsupported;

type FResult<T> = Result<T, Unsupported>;

/// Block collection kinds tracked on the indent stack.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Mapping,
    Sequence,
}

enum State {
    /// Nothing emitted yet.
    Start,
    /// Inside the document; block structure tracked by `stack`.
    Body,
    /// Document closed; emit `StreamEnd` next.
    Closing,
    /// Everything emitted.
    Done,
}

pub(super) struct LineScanner<'a> {
    text: &'a str,
    /// Byte offset of the next unread character.
    pos: usize,
    /// Events produced by the last processed line, drained in order.
    queue: std::collections::VecDeque<Event<'a>>,
    /// Open block collections as (indent, kind).
    stack: Vec<(usize, Kind)>,
    /// Set after a `key:` line whose value, if any, is a nested block
    /// collection introduced by the next line's indentation.
    pending_child: Option<usize>,
    state: State,
}

impl<'a> LineScanner<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            // A line yields at most a handful of events (flow mappings emit
            // start + members + end).
            queue: std::collections::VecDeque::with_capacity(16),
            stack: Vec::new(),
            pending_child: None,
            state: State::Start,
        }
    }

    pub(super) fn next_event(&mut self) -> FResult<Event<'a>> {
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Ok(ev);
            }
            match self.state {
                State::Done => return Err(Unsupported::here()),
                State::Closing => {
                    self.state = State::Done;
                    return Ok(Event::StreamEnd);
                }
                State::Start | State::Body => self.advance()?,
            }
        }
    }

    /// Consume input lines until at least one event is queued or the
    /// document is finished.
    fn advance(&mut self) -> FResult<()> {
        let Some((indent, content)) = self.next_content_line()? else {
            // EOF: resolve any pending empty value, close open collections
            // and the document.
            if matches!(self.state, State::Start) {
                // Empty input isn't a mapping; let the fallback error.
                return Err(Unsupported::here());
            }
            if self.pending_child.take().is_some() {
                self.queue.push_back(null_scalar());
            }
            while let Some((_, kind)) = self.stack.pop() {
                self.queue.push_back(end_event(kind));
            }
            self.queue.push_back(Event::DocumentEnd);
            self.state = State::Closing;
            return Ok(());
        };

        if matches!(self.state, State::Start) {
            if indent != 0 || is_document_marker(content) {
                // Leading documents / weird openings: serde handles them.
                return Err(Unsupported::here());
            }
            self.queue.push_back(Event::StreamStart);
            self.queue.push_back(Event::DocumentStart(false));
            self.queue.push_back(Event::MappingStart(0, None));
            self.stack.push((0, Kind::Mapping));
            self.state = State::Body;
        } else if is_document_marker(content) {
            // Multi-document input.
            return Err(Unsupported::here());
        }

        // A `key:` line opens a child collection only if the next content
        // line is more deeply indented (or is a sequence item at the key's
        // own indent). Otherwise the value was an empty scalar.
        if let Some(key_indent) = self.pending_child.take() {
            if indent > key_indent {
                let kind = if is_sequence_item(content) {
                    Kind::Sequence
                } else {
                    Kind::Mapping
                };
                self.queue.push_back(start_event(kind));
                self.stack.push((indent, kind));
            } else if indent == key_indent && is_sequence_item(content) {
                self.queue.push_back(start_event(Kind::Sequence));
                self.stack.push((indent, Kind::Sequence));
            } else {
                self.queue.push_back(null_scalar());
            }
        }

        // Close collections the line has dedented out of. A sequence living
        // at the same indent as its parent mapping's keys also closes when a
        // non-item line appears at that indent.
        while let Some(&(open_indent, kind)) = self.stack.last() {
            if indent < open_indent
                || (indent == open_indent && kind == Kind::Sequence && !is_sequence_item(content))
            {
                self.queue.push_back(end_event(kind));
                self.stack.pop();
            } else {
                break;
            }
        }

        let Some(&(open_indent, kind)) = self.stack.last() else {
            return Err(Unsupported::here());
        };
        if indent != open_indent {
            // A line deeper than the open collection with no pending key is
            // a multiline plain scalar or an indentation we don't model.
            return Err(Unsupported::here());
        }

        match kind {
            Kind::Mapping => self.key_line(indent, content),
            Kind::Sequence => self.sequence_item(content),
        }
    }

    /// Next non-blank, non-comment line as (indent, content). Content
    /// excludes indentation and the trailing newline. pnpm never emits
    /// comments; full-line comments are skipped anyway since dropping them
    /// preserves meaning.
    fn next_content_line(&mut self) -> FResult<Option<(usize, &'a str)>> {
        loop {
            if self.pos >= self.text.len() {
                return Ok(None);
            }
            let rest = &self.text[self.pos..];
            let (line, next_pos) = match memchr::memchr(b'\n', rest.as_bytes()) {
                Some(i) => (&rest[..i], self.pos + i + 1),
                None => (rest, self.text.len()),
            };
            self.pos = next_pos;

            if line.contains('\r') {
                // CRLF input: saphyr handles it, we don't model it.
                return Err(Unsupported::here());
            }
            let indent = count_indent(line);
            let content = &line[indent..];
            if content.is_empty() {
                continue;
            }
            if content.as_bytes()[0] == b'\t' {
                return Err(Unsupported::here());
            }
            if content.starts_with('#') {
                continue;
            }
            return Ok(Some((indent, content)));
        }
    }

    fn key_line(&mut self, indent: usize, content: &'a str) -> FResult<()> {
        let (key_event, rest) = split_key(content)?;
        self.queue.push_back(key_event);

        let value = rest.trim_start_matches(' ');
        if value.is_empty() {
            self.pending_child = Some(indent);
            return Ok(());
        }
        self.value_events(indent, value)
    }

    fn sequence_item(&mut self, content: &'a str) -> FResult<()> {
        let Some(item) = content.strip_prefix("- ") else {
            return Err(Unsupported::here());
        };
        let item = item.trim_start_matches(' ');
        // Items are scalars only in the pnpm subset; nested collections
        // under `-` leave the fast path.
        let ev = inline_scalar(item, FlowContext::Block)?;
        self.queue.push_back(ev);
        Ok(())
    }

    /// Emit events for a value that starts on the key's own line.
    fn value_events(&mut self, key_indent: usize, value: &'a str) -> FResult<()> {
        match value.as_bytes()[0] {
            b'{' => self.flow_mapping(value),
            b'[' => self.flow_sequence(value),
            b'>' | b'|' => self.block_scalar(key_indent, value),
            _ => {
                let ev = inline_scalar(value, FlowContext::Block)?;
                self.queue.push_back(ev);
                Ok(())
            }
        }
    }

    /// One-line flow mapping: `{a: b, c: 'd'}`. Nested collections bail.
    fn flow_mapping(&mut self, value: &'a str) -> FResult<()> {
        let inner = value
            .strip_prefix('{')
            .and_then(|v| v.strip_suffix('}'))
            .ok_or_else(Unsupported::here)?;
        self.queue.push_back(Event::MappingStart(0, None));
        for member in split_flow_members(inner)? {
            let member = member.trim_matches(' ');
            if member.is_empty() {
                return Err(Unsupported::here());
            }
            let (key_event, rest) = split_key(member)?;
            let member_value = rest.trim_start_matches(' ');
            if member_value.is_empty() {
                return Err(Unsupported::here());
            }
            self.queue.push_back(key_event);
            self.queue
                .push_back(inline_scalar(member_value, FlowContext::Flow)?);
        }
        self.queue.push_back(Event::MappingEnd);
        Ok(())
    }

    /// One-line flow sequence of scalars: `[darwin, linux]`.
    fn flow_sequence(&mut self, value: &'a str) -> FResult<()> {
        let inner = value
            .strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .ok_or_else(Unsupported::here)?;
        self.queue.push_back(Event::SequenceStart(0, None));
        for member in split_flow_members(inner)? {
            let member = member.trim_matches(' ');
            if member.is_empty() {
                return Err(Unsupported::here());
            }
            self.queue
                .push_back(inline_scalar(member, FlowContext::Flow)?);
        }
        self.queue.push_back(Event::SequenceEnd);
        Ok(())
    }

    /// Block scalar (`>`, `>-`, `|`, `|-`), the shapes pnpm emits for long
    /// `deprecated:` messages. Folded (`>`) blocks fold line breaks into
    /// spaces; literal (`|`) blocks keep them. Explicit indentation
    /// indicators, `keep` (`+`) chomping, leading/trailing blank lines, and
    /// more-indented lines under folding all bail: their rules are subtle
    /// enough that the general parser should decide. (Literal blocks keep
    /// more-indented lines verbatim, so those are safe to accept.)
    fn block_scalar(&mut self, key_indent: usize, value: &'a str) -> FResult<()> {
        let (literal, chomp_strip) = match value {
            ">" => (false, false),
            ">-" => (false, true),
            "|" => (true, false),
            "|-" => (true, true),
            _ => return Err(Unsupported::here()),
        };

        let mut result = String::new();
        let mut block_indent: Option<usize> = None;
        let mut pending_breaks = 0usize;
        let mut saw_content = false;

        loop {
            let rest = &self.text[self.pos..];
            if rest.is_empty() {
                break;
            }
            let (line, next_pos) = match memchr::memchr(b'\n', rest.as_bytes()) {
                Some(i) => (&rest[..i], self.pos + i + 1),
                None => (rest, self.text.len()),
            };
            if line.contains('\r') || line.contains('\t') {
                return Err(Unsupported::here());
            }
            let indent = count_indent(line);
            let content = &line[indent..];

            if content.is_empty() {
                if !saw_content {
                    // Leading blank lines interact with block indent
                    // detection in ways we don't model.
                    return Err(Unsupported::here());
                }
                pending_breaks += 1;
                self.pos = next_pos;
                continue;
            }
            if indent <= key_indent {
                // Block ended at the previous line.
                break;
            }
            let expected = *block_indent.get_or_insert(indent);
            if indent < expected || (indent > expected && !literal) {
                // Less-indented content is invalid; more-indented lines are
                // kept literally under folding — leave those to the general
                // parser. Literal blocks keep them verbatim below.
                return Err(Unsupported::here());
            }
            let text = &line[expected..];
            if literal && text.ends_with(' ') {
                // Trailing whitespace preservation in literal blocks is a
                // corner we'd rather not mirror by hand.
                return Err(Unsupported::here());
            }
            if saw_content {
                if literal {
                    for _ in 0..=pending_breaks {
                        result.push('\n');
                    }
                } else if pending_breaks > 0 {
                    for _ in 0..pending_breaks {
                        result.push('\n');
                    }
                } else {
                    result.push(' ');
                }
            }
            pending_breaks = 0;
            result.push_str(if literal {
                text
            } else {
                text.trim_end_matches(' ')
            });
            saw_content = true;
            self.pos = next_pos;
        }

        if !saw_content {
            // Empty blocks hit chomping subtleties; bail. Trailing blank
            // lines (`pending_breaks` left over) are safe: both strip and
            // clip chomping discard trailing breaks, and `keep` (`+`)
            // already bailed at the indicator.
            return Err(Unsupported::here());
        }
        if !chomp_strip {
            result.push('\n');
        }
        let style = if literal {
            ScalarStyle::Literal
        } else {
            ScalarStyle::Folded
        };
        self.queue
            .push_back(Event::Scalar(Cow::Owned(result), style, 0, None));
        Ok(())
    }
}

fn count_indent(line: &str) -> usize {
    line.as_bytes().iter().take_while(|&&b| b == b' ').count()
}

fn is_document_marker(content: &str) -> bool {
    content == "---"
        || content == "..."
        || content.starts_with("--- ")
        || content.starts_with("... ")
}

fn is_sequence_item(content: &str) -> bool {
    content.starts_with("- ")
}

fn null_scalar() -> Event<'static> {
    // What saphyr emits for a key with no value.
    Event::Scalar(Cow::Borrowed("~"), ScalarStyle::Plain, 0, None)
}

fn start_event(kind: Kind) -> Event<'static> {
    match kind {
        Kind::Mapping => Event::MappingStart(0, None),
        Kind::Sequence => Event::SequenceStart(0, None),
    }
}

fn end_event(kind: Kind) -> Event<'static> {
    match kind {
        Kind::Mapping => Event::MappingEnd,
        Kind::Sequence => Event::SequenceEnd,
    }
}

/// Split `key: rest` or `key:` (end of line), returning the key's scalar
/// event and the remainder after the colon.
fn split_key(content: &str) -> FResult<(Event<'_>, &str)> {
    match content.as_bytes()[0] {
        b'\'' => {
            let (event, rest) = single_quoted(content)?;
            let rest = rest.strip_prefix(':').ok_or_else(Unsupported::here)?;
            if !rest.is_empty() && !rest.starts_with(' ') {
                return Err(Unsupported::here());
            }
            Ok((event, rest))
        }
        b'"' => {
            let (event, rest) = double_quoted(content)?;
            let rest = rest.strip_prefix(':').ok_or_else(Unsupported::here)?;
            if !rest.is_empty() && !rest.starts_with(' ') {
                return Err(Unsupported::here());
            }
            Ok((event, rest))
        }
        _ => {
            // Plain key: ends at the first `:` that is followed by a space
            // or the end of the line.
            let bytes = content.as_bytes();
            let mut search_from = 0;
            loop {
                let Some(i) = memchr::memchr(b':', &bytes[search_from..]) else {
                    return Err(Unsupported::here());
                };
                let colon = search_from + i;
                if colon + 1 == bytes.len() || bytes[colon + 1] == b' ' {
                    let key = &content[..colon];
                    plain_scalar_check(key, FlowContext::Block)?;
                    return Ok((
                        Event::Scalar(Cow::Borrowed(key), ScalarStyle::Plain, 0, None),
                        &content[colon + 1..],
                    ));
                }
                search_from = colon + 1;
            }
        }
    }
}

enum FlowContext {
    Block,
    Flow,
}

/// Scalar occupying the remainder of a line (or a flow member): plain,
/// single-quoted, or double-quoted.
fn inline_scalar(value: &str, ctx: FlowContext) -> FResult<Event<'_>> {
    match value.as_bytes()[0] {
        b'\'' => {
            let (event, rest) = single_quoted(value)?;
            if !rest.trim_matches(' ').is_empty() {
                return Err(Unsupported::here());
            }
            Ok(event)
        }
        b'"' => {
            let (event, rest) = double_quoted(value)?;
            if !rest.trim_matches(' ').is_empty() {
                return Err(Unsupported::here());
            }
            Ok(event)
        }
        _ => {
            let value = value.trim_end_matches(' ');
            plain_scalar_check(value, ctx)?;
            Ok(Event::Scalar(
                Cow::Borrowed(value),
                ScalarStyle::Plain,
                0,
                None,
            ))
        }
    }
}

/// Reject plain scalars whose meaning we can't guarantee matches a general
/// YAML scanner: indicator-led strings, embedded `: `/` #`, trailing `:`,
/// and (in flow context) flow delimiters.
fn plain_scalar_check(value: &str, ctx: FlowContext) -> FResult<()> {
    if value.is_empty() {
        return Err(Unsupported::here());
    }
    let bytes = value.as_bytes();
    match bytes[0] {
        b'!' | b'&' | b'*' | b'|' | b'>' | b'%' | b'@' | b'`' | b'\'' | b'"' | b'{' | b'}'
        | b'[' | b']' | b',' | b'#' => return Err(Unsupported::here()),
        b'-' | b'?' | b':' if bytes.len() == 1 || bytes[1] == b' ' => {
            return Err(Unsupported::here());
        }
        _ => {}
    }
    if bytes.contains(&b'\t') {
        return Err(Unsupported::here());
    }
    if value.ends_with(':') || value.contains(": ") || value.contains(" #") {
        return Err(Unsupported::here());
    }
    if matches!(ctx, FlowContext::Flow)
        && bytes
            .iter()
            .any(|b| matches!(b, b'{' | b'}' | b'[' | b']' | b','))
    {
        return Err(Unsupported::here());
    }
    Ok(())
}

/// Parse a single-quoted scalar at the start of `s`, returning its event
/// and the remainder. `''` escapes force an owned string; otherwise the
/// content is borrowed.
fn single_quoted(s: &str) -> FResult<(Event<'_>, &str)> {
    let inner = &s[1..];
    let bytes = inner.as_bytes();
    let mut i = 0;
    let mut has_escape = false;
    loop {
        let Some(q) = memchr::memchr(b'\'', &bytes[i..]) else {
            // Closing quote on another line: multiline quoted scalar.
            return Err(Unsupported::here());
        };
        let q = i + q;
        if bytes.get(q + 1) == Some(&b'\'') {
            has_escape = true;
            i = q + 2;
            continue;
        }
        let raw = &inner[..q];
        if raw.contains('\t') {
            return Err(Unsupported::here());
        }
        let value = if has_escape {
            Cow::Owned(raw.replace("''", "'"))
        } else {
            Cow::Borrowed(raw)
        };
        return Ok((
            Event::Scalar(value, ScalarStyle::SingleQuoted, 0, None),
            &inner[q + 1..],
        ));
    }
}

/// Parse a double-quoted scalar at the start of `s`. Backslash escapes
/// bail: pnpm doesn't emit them and their decoding table is long.
fn double_quoted(s: &str) -> FResult<(Event<'_>, &str)> {
    let inner = &s[1..];
    let bytes = inner.as_bytes();
    let Some(q) = memchr::memchr(b'"', bytes) else {
        return Err(Unsupported::here());
    };
    let raw = &inner[..q];
    if raw.contains('\\') || raw.contains('\t') {
        return Err(Unsupported::here());
    }
    Ok((
        Event::Scalar(Cow::Borrowed(raw), ScalarStyle::DoubleQuoted, 0, None),
        &inner[q + 1..],
    ))
}

/// Split the inside of a one-line flow collection on top-level commas,
/// respecting quoted members. Nested flow collections bail.
fn split_flow_members(inner: &str) -> FResult<Vec<&str>> {
    let inner = inner.trim_matches(' ');
    let mut members = Vec::new();
    if inner.is_empty() {
        return Ok(members);
    }
    let bytes = inner.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                // Skip to the closing quote, honoring '' escapes.
                i += 1;
                loop {
                    let Some(q) = memchr::memchr(b'\'', &bytes[i..]) else {
                        return Err(Unsupported::here());
                    };
                    i += q;
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                }
            }
            b'"' => {
                i += 1;
                let Some(q) = memchr::memchr(b'"', &bytes[i..]) else {
                    return Err(Unsupported::here());
                };
                if bytes[i..i + q].contains(&b'\\') {
                    return Err(Unsupported::here());
                }
                i += q + 1;
            }
            b'{' | b'[' => return Err(Unsupported::here()),
            b',' => {
                members.push(&inner[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    members.push(&inner[start..]);
    Ok(members)
}
