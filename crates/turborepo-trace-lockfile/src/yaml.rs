//! A zero-copy parser for the YAML subset that lockfile writers emit.
//!
//! Supported: block mappings and sequences (including compact `- key: value`
//! items), flow mappings and sequences (possibly spanning lines), plain,
//! single- and double-quoted scalars, literal/folded block scalars, comments,
//! CRLF line endings, `---` document separators, and explicit keys with a
//! scalar key (`? key` on one line, `: value` on the next: what js-yaml, and so
//! pnpm, writes for a key longer than 1024 characters, such as a snapshot id
//! with a deep peer suffix).
//! Not supported (rejected or read literally): anchors and aliases, tags,
//! explicit keys that are collections, tabs in indentation. All scalars are
//! returned as strings; no type resolution happens.
//!
//! The parser is streaming at the mapping-entry level: [`Parser::entries`]
//! parses one entry value at a time into a small [`Node`] tree, so a 20 MB
//! lockfile never materializes as one tree. Scalars borrow from the input
//! unless they contain escapes.

use std::borrow::Cow;

/// A parsed YAML value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node<'a> {
    /// Empty value (`key:` with nothing nested).
    Null,
    /// Any scalar, as text.
    Scalar(Cow<'a, str>),
    /// A mapping, in document order.
    Map(Vec<(Cow<'a, str>, Node<'a>)>),
    /// A sequence.
    Seq(Vec<Node<'a>>),
}

impl<'a> Node<'a> {
    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Node<'a>> {
        match self {
            Node::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Node::as_str)
    }

    pub(crate) fn entries(&self) -> &[(Cow<'a, str>, Node<'a>)] {
        match self {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }

    /// Scalars of a sequence; a single scalar counts as a one-element list.
    pub(crate) fn str_list(&self) -> Vec<String> {
        match self {
            Node::Seq(items) => items
                .iter()
                .filter_map(|n| n.as_str().map(str::to_string))
                .collect(),
            Node::Scalar(s) => vec![s.to_string()],
            _ => Vec::new(),
        }
    }

    pub(crate) fn is_true(&self) -> bool {
        self.as_str() == Some("true")
    }
}

/// A syntax error with a 1-based line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YamlError {
    pub(crate) line: usize,
    pub(crate) message: String,
}

pub(crate) type Result<T> = std::result::Result<T, YamlError>;

/// How the value of an explicit key (`? key`) is written.
enum ExplicitValue {
    /// No `: value` line: the value is null.
    Missing,
    /// `: key: value`: a block mapping with its keys at this column.
    CompactMap(usize),
    /// `: value` on the line (a scalar or a flow collection), or a block below
    /// it.
    Inline,
}

/// Line-oriented recursive-descent parser over one document.
pub(crate) struct Parser<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    /// `(pos, indent)` of the last [`Parser::peek_content_line`] result: the
    /// same line is peeked several times while deciding what a value is.
    peeked: Option<(usize, Option<usize>)>,
    /// Set after an explicit key (`? key`): the cursor position its value
    /// starts at, and whether a `: value` line follows. The next
    /// [`Parser::value`] or [`Parser::entries`] at that position reads the
    /// value in the explicit form.
    explicit: Option<(usize, bool)>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(src: &'a str) -> Self {
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        Self {
            src,
            b: src.as_bytes(),
            pos: 0,
            peeked: None,
            explicit: None,
        }
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T> {
        let line = 1 + memchr::memchr_iter(b'\n', &self.b[..self.pos.min(self.b.len())]).count();
        Err(YamlError {
            line,
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn line_end(&self, from: usize) -> usize {
        memchr::memchr(b'\n', &self.b[from..]).map_or(self.b.len(), |i| from + i)
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }

    fn at_eol(&self) -> bool {
        matches!(self.peek(), None | Some(b'\n' | b'\r'))
    }

    /// Consumes the rest of the current line, which must be blank or a comment.
    fn finish_line(&mut self) -> Result<()> {
        self.skip_spaces();
        match self.peek() {
            None => Ok(()),
            Some(b'#') | Some(b'\n') | Some(b'\r') => {
                self.pos = (self.line_end(self.pos) + 1).min(self.b.len());
                Ok(())
            }
            Some(_) => self.err("unexpected trailing content"),
        }
    }

    /// Moves to the start of the next line with content (skipping blank and
    /// comment lines) and returns its indentation, without consuming it.
    /// `None` at end of document (EOF or `---`).
    fn peek_content_line(&mut self) -> Result<Option<usize>> {
        if let Some((at, indent)) = self.peeked
            && at == self.pos
        {
            return Ok(indent);
        }
        let r = self.scan_content_line()?;
        self.peeked = Some((self.pos, r));
        Ok(r)
    }

    fn scan_content_line(&mut self) -> Result<Option<usize>> {
        loop {
            if self.pos >= self.b.len() {
                return Ok(None);
            }
            let start = self.pos;
            let mut i = start;
            while i < self.b.len() && self.b[i] == b' ' {
                i += 1;
            }
            match self.b.get(i) {
                None => {
                    self.pos = i;
                    return Ok(None);
                }
                Some(b'\n') | Some(b'#') => self.pos = (self.line_end(i) + 1).min(self.b.len()),
                Some(b'\r') if self.b.get(i + 1) == Some(&b'\n') => self.pos = i + 2,
                Some(b'\t') => {
                    self.pos = i;
                    return self.err("tab in indentation");
                }
                Some(_) => {
                    if i == start && self.is_doc_marker(start) {
                        return Ok(None);
                    }
                    return Ok(Some(i - start));
                }
            }
        }
    }

    fn is_doc_marker(&self, at: usize) -> bool {
        let rest = &self.b[at..];
        (rest.starts_with(b"---") || rest.starts_with(b"..."))
            && matches!(rest.get(3), None | Some(b'\n' | b'\r' | b' '))
    }

    /// Skips a leading `---` marker (and `%` directives) at the start of a
    /// document.
    fn skip_doc_start(&mut self) -> Result<()> {
        while self.peek_content_line()?.is_none() && self.pos < self.b.len() {
            // At a `---`/`...` line: skip it.
            self.pos = (self.line_end(self.pos) + 1).min(self.b.len());
        }
        Ok(())
    }

    /// Number of `---`-separated documents with content. Only marker lines are
    /// inspected, so this is a `memmem` scan, not a line-by-line pass.
    pub(crate) fn document_count(src: &str) -> usize {
        let b = src.as_bytes();
        let is_marker_at = |i: usize| {
            b[i..].starts_with(b"---") && matches!(b.get(i + 3), None | Some(b'\n' | b'\r' | b' '))
        };
        let mut markers: Vec<usize> = memchr::memmem::find_iter(b, b"\n---")
            .map(|i| i + 1)
            .filter(|&i| is_marker_at(i))
            .collect();
        if is_marker_at(0) {
            markers.insert(0, 0);
        }
        // Segments between markers; a segment counts when it has a non-comment content
        // line.
        let has_content = |seg: &[u8]| {
            seg.split(|&c| c == b'\n').any(|line| {
                let t = line.trim_ascii();
                !t.is_empty() && t[0] != b'#'
            })
        };
        let mut n = 0;
        let mut start = 0;
        for &m in &markers {
            if has_content(&b[start..m]) {
                n += 1;
            }
            start = (m + 3).min(b.len());
        }
        n + usize::from(has_content(&b[start..]))
    }

    /// Positions the parser at the start of document `index` (0-based).
    pub(crate) fn seek_document(&mut self, index: usize) -> Result<()> {
        self.pos = 0;
        for _ in 0..index {
            self.skip_doc_start()?;
            // Skip the document body to the next marker.
            loop {
                if self.pos >= self.b.len() {
                    return self.err("document index out of range");
                }
                if self.is_doc_marker(self.pos) && self.b[self.pos] == b'-' {
                    break;
                }
                self.pos = (self.line_end(self.pos) + 1).min(self.b.len());
            }
        }
        self.skip_doc_start()
    }

    /// Iterates the top-level mapping of the current document. `f` receives
    /// each key with the cursor right after `key:`, and must consume the
    /// value with [`Parser::value`] or [`Parser::entries`] (parent indent
    /// 0).
    pub(crate) fn top_level(
        &mut self,
        mut f: impl FnMut(&mut Self, Cow<'a, str>) -> Result<()>,
    ) -> Result<()> {
        self.skip_doc_start()?;
        self.map_body(0, false, &mut |p, k| f(p, k))
    }

    /// Streams a block mapping value: calls `f(key, value)` for each entry of
    /// the mapping nested under the current key. Accepts `{}` and flow
    /// mappings too.
    pub(crate) fn entries(
        &mut self,
        parent_indent: usize,
        mut f: impl FnMut(Cow<'a, str>, Node<'a>) -> Result<()>,
    ) -> Result<()> {
        match self.explicit_value() {
            Some(ExplicitValue::Missing) => return Ok(()),
            Some(ExplicitValue::CompactMap(col)) => {
                return self.map_body(col, true, &mut |p, k| {
                    let v = p.value(col)?;
                    f(k, v)
                });
            }
            Some(ExplicitValue::Inline) | None => {}
        }
        self.skip_spaces();
        if self.peek() == Some(b'{') {
            let node = self.flow_value()?;
            self.finish_line()?;
            if let Node::Map(entries) = node {
                for (k, v) in entries {
                    f(k, v)?;
                }
            }
            return Ok(());
        }
        if !self.at_eol() && self.peek() != Some(b'#') {
            // A scalar where a mapping was expected: consume it and yield nothing.
            let _ = self.value(parent_indent)?;
            return Ok(());
        }
        self.finish_line()?;
        let Some(ind) = self.peek_content_line()? else {
            return Ok(());
        };
        if ind <= parent_indent {
            return Ok(());
        }
        self.map_body(ind, false, &mut |p, k| {
            let v = p.value(ind)?;
            f(k, v)
        })
    }

    /// Parses the block mapping whose keys sit at column `indent`. When
    /// `inline_first` is set the cursor is already at the first key
    /// (compact mapping inside a sequence item).
    fn map_body(
        &mut self,
        indent: usize,
        mut inline_first: bool,
        f: &mut dyn FnMut(&mut Self, Cow<'a, str>) -> Result<()>,
    ) -> Result<()> {
        loop {
            if !inline_first {
                let Some(ind) = self.peek_content_line()? else {
                    return Ok(());
                };
                if ind < indent {
                    return Ok(());
                }
                if ind > indent {
                    self.pos += ind;
                    return self.err(format!(
                        "unexpected indentation (expected {indent}, got {ind})"
                    ));
                }
                self.pos += ind;
            }
            inline_first = false;
            if self.peek() == Some(b'-')
                && matches!(self.b.get(self.pos + 1), None | Some(b' ' | b'\n' | b'\r'))
            {
                if indent == 0 {
                    return self.err("sequence item where a mapping key was expected");
                }
                // A sequence at the parent's indentation ends this mapping.
                return Ok(());
            }
            let key = if self.peek() == Some(b'?') && matches!(self.b.get(self.pos + 1), Some(b' '))
            {
                self.explicit_key(indent)?
            } else {
                self.key()?
            };
            f(self, key)?;
        }
    }

    /// An explicit key: `? key` (cursor at the `?`), with its value on the next
    /// line at the same indentation, after `: `. Leaves the cursor where
    /// the value starts and records the form in `self.explicit` for the
    /// value parser.
    fn explicit_key(&mut self, indent: usize) -> Result<Cow<'a, str>> {
        self.pos += 1;
        self.skip_spaces();
        let key = match self.peek() {
            Some(b'\'') => self.single_quoted()?,
            Some(b'"') => self.double_quoted()?,
            Some(b'{' | b'[' | b'-' | b'|' | b'>' | b'&' | b'*' | b'!' | b'\n' | b'\r') | None => {
                return self.err("explicit keys must be scalars on the line of the '?'");
            }
            _ => Cow::Borrowed(self.plain_line()),
        };
        self.finish_line()?;
        let has_value = match self.peek_content_line()? {
            Some(ind) if ind == indent => {
                let at = self.pos + ind;
                self.b[at] == b':'
                    && matches!(self.b.get(at + 1), None | Some(b' ' | b'\n' | b'\r'))
            }
            _ => false,
        };
        if has_value {
            self.pos += indent + 1;
        }
        self.explicit = Some((self.pos, has_value));
        Ok(key)
    }

    /// The form of the value of an explicit key, when the cursor is where
    /// [`Parser::explicit_key`] left it.
    fn explicit_value(&mut self) -> Option<ExplicitValue> {
        let (at, has_value) = self.explicit.take()?;
        if at != self.pos {
            return None;
        }
        if !has_value {
            return Some(ExplicitValue::Missing);
        }
        self.skip_spaces();
        // `: key: value` starts a block mapping whose keys sit at the column after `:
        // `.
        if !self.at_eol() && self.peek() != Some(b'#') && self.looks_like_key() {
            return Some(ExplicitValue::CompactMap(
                self.pos - self.line_start(self.pos),
            ));
        }
        Some(ExplicitValue::Inline)
    }

    fn key(&mut self) -> Result<Cow<'a, str>> {
        let key = match self.peek() {
            Some(b'\'') => self.single_quoted()?,
            Some(b'"') => self.double_quoted()?,
            Some(b'&' | b'*' | b'!') => {
                return self.err("anchors, aliases and tags are not supported");
            }
            _ => {
                let start = self.pos;
                loop {
                    match self.peek() {
                        None | Some(b'\n') | Some(b'\r') => {
                            return self.err("expected ':' after key");
                        }
                        Some(b':')
                            if matches!(
                                self.b.get(self.pos + 1),
                                None | Some(b' ' | b'\n' | b'\r')
                            ) =>
                        {
                            break;
                        }
                        _ => self.pos += 1,
                    }
                }
                Cow::Borrowed(self.src[start..self.pos].trim_end())
            }
        };
        self.skip_spaces();
        if self.peek() != Some(b':') {
            return self.err("expected ':' after key");
        }
        self.pos += 1;
        Ok(key)
    }

    /// Parses the value after `key:` (cursor right after the colon).
    /// `parent_indent` is the column of the key.
    pub(crate) fn value(&mut self, parent_indent: usize) -> Result<Node<'a>> {
        match self.explicit_value() {
            Some(ExplicitValue::Missing) => return Ok(Node::Null),
            Some(ExplicitValue::CompactMap(col)) => {
                let mut entries = Vec::new();
                self.map_body(col, true, &mut |p, k| {
                    let v = p.value(col)?;
                    entries.push((k, v));
                    Ok(())
                })?;
                return Ok(Node::Map(entries));
            }
            Some(ExplicitValue::Inline) | None => {}
        }
        self.skip_spaces();
        match self.peek() {
            None | Some(b'\n') | Some(b'\r') | Some(b'#') => {
                self.finish_line()?;
                self.block_child(parent_indent)
            }
            Some(b'{') | Some(b'[') => {
                let n = self.flow_value()?;
                self.finish_line()?;
                Ok(n)
            }
            Some(b'\'') => {
                let s = self.single_quoted()?;
                self.finish_line()?;
                Ok(Node::Scalar(s))
            }
            Some(b'"') => {
                let s = self.double_quoted()?;
                self.finish_line()?;
                Ok(Node::Scalar(s))
            }
            Some(b'|') | Some(b'>') => self.block_scalar(parent_indent),
            Some(b'&' | b'*' | b'!') => self.err("anchors, aliases and tags are not supported"),
            _ => self.plain_block_scalar(parent_indent),
        }
    }

    fn block_child(&mut self, parent_indent: usize) -> Result<Node<'a>> {
        let Some(ind) = self.peek_content_line()? else {
            return Ok(Node::Null);
        };
        let at = self.pos + ind;
        let is_seq =
            self.b[at] == b'-' && matches!(self.b.get(at + 1), None | Some(b' ' | b'\n' | b'\r'));
        if ind > parent_indent || (is_seq && ind == parent_indent && parent_indent > 0) {
            if is_seq {
                return self.seq_body(ind);
            }
            if ind > parent_indent {
                let mut entries = Vec::new();
                self.map_body(ind, false, &mut |p, k| {
                    let v = p.value(ind)?;
                    entries.push((k, v));
                    Ok(())
                })?;
                return Ok(Node::Map(entries));
            }
        }
        Ok(Node::Null)
    }

    fn seq_body(&mut self, indent: usize) -> Result<Node<'a>> {
        let mut items = Vec::new();
        while let Some(ind) = self.peek_content_line()? {
            if ind != indent {
                break;
            }
            let at = self.pos + ind;
            if !(self.b[at] == b'-'
                && matches!(self.b.get(at + 1), None | Some(b' ' | b'\n' | b'\r')))
            {
                break;
            }
            self.pos = at + 1;
            self.skip_spaces();
            if self.at_eol() || self.peek() == Some(b'#') {
                self.finish_line()?;
                items.push(self.block_child(indent)?);
                continue;
            }
            if self.looks_like_key() {
                let col = self.pos - self.line_start(self.pos);
                let mut entries = Vec::new();
                self.map_body(col, true, &mut |p, k| {
                    let v = p.value(col)?;
                    entries.push((k, v));
                    Ok(())
                })?;
                items.push(Node::Map(entries));
            } else {
                items.push(self.value(indent)?);
            }
        }
        Ok(Node::Seq(items))
    }

    fn line_start(&self, at: usize) -> usize {
        memchr::memrchr(b'\n', &self.b[..at]).map_or(0, |i| i + 1)
    }

    /// Whether the rest of the line starts with a mapping key (`key: …` or
    /// `key:`).
    fn looks_like_key(&self) -> bool {
        let end = self.line_end(self.pos);
        let line = &self.b[self.pos..end];
        match line.first() {
            Some(b'{' | b'[' | b'|' | b'>') => false,
            Some(&q @ (b'\'' | b'"')) => {
                // Quoted key if the closing quote is followed by ':'.
                let mut i = 1;
                while i < line.len() {
                    if line[i] == b'\\' && q == b'"' {
                        i += 2;
                        continue;
                    }
                    if line[i] == q {
                        if q == b'\'' && line.get(i + 1) == Some(&b'\'') {
                            i += 2;
                            continue;
                        }
                        let rest = &line[i + 1..];
                        let rest = rest.trim_ascii_start();
                        return rest.first() == Some(&b':');
                    }
                    i += 1;
                }
                false
            }
            _ => line.windows(2).any(|w| w == b": ") || line.trim_ascii_end().ends_with(b":"),
        }
    }

    fn single_quoted(&mut self) -> Result<Cow<'a, str>> {
        debug_assert_eq!(self.peek(), Some(b'\''));
        self.pos += 1;
        let start = self.pos;
        let mut owned: Option<String> = None;
        loop {
            let Some(rel) = memchr::memchr(b'\'', &self.b[self.pos..]) else {
                return self.err("unterminated single-quoted string");
            };
            let q = self.pos + rel;
            if self.b.get(q + 1) == Some(&b'\'') {
                let buf = owned.get_or_insert_with(String::new);
                buf.push_str(&self.src[self.pos..=q]);
                self.pos = q + 2;
                continue;
            }
            let s = match owned {
                Some(mut buf) => {
                    buf.push_str(&self.src[self.pos..q]);
                    Cow::Owned(buf)
                }
                None => Cow::Borrowed(&self.src[start..q]),
            };
            self.pos = q + 1;
            if s.contains('\n') {
                return Ok(Cow::Owned(fold_lines(&s)));
            }
            return Ok(s);
        }
    }

    fn double_quoted(&mut self) -> Result<Cow<'a, str>> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.pos += 1;
        let start = self.pos;
        // Fast path: no escapes.
        let Some(rel) = memchr::memchr2(b'"', b'\\', &self.b[self.pos..]) else {
            return self.err("unterminated double-quoted string");
        };
        if self.b[self.pos + rel] == b'"' {
            let s = &self.src[start..self.pos + rel];
            self.pos += rel + 1;
            return Ok(if s.contains('\n') {
                Cow::Owned(fold_lines(s))
            } else {
                Cow::Borrowed(s)
            });
        }
        let mut out = String::new();
        loop {
            let Some(rel) = memchr::memchr2(b'"', b'\\', &self.b[self.pos..]) else {
                return self.err("unterminated double-quoted string");
            };
            out.push_str(&self.src[self.pos..self.pos + rel]);
            self.pos += rel;
            if self.b[self.pos] == b'"' {
                self.pos += 1;
                return Ok(Cow::Owned(if out.contains('\n') {
                    fold_lines(&out)
                } else {
                    out
                }));
            }
            // Escape.
            self.pos += 1;
            let Some(c) = self.peek() else {
                return self.err("unterminated escape");
            };
            self.pos += 1;
            match c {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'r' => out.push('\r'),
                b'0' => out.push('\0'),
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b' ' => out.push(' '),
                b'\n' => {}
                b'x' | b'u' | b'U' => {
                    let len = match c {
                        b'x' => 2,
                        b'u' => 4,
                        _ => 8,
                    };
                    let hex = self.src.get(self.pos..self.pos + len).unwrap_or("");
                    let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    else {
                        return self.err("invalid escape");
                    };
                    out.push(ch);
                    self.pos += len;
                }
                _ => return self.err("unknown escape"),
            }
        }
    }

    /// A plain scalar in block context, possibly continued on more-indented
    /// lines.
    fn plain_block_scalar(&mut self, parent_indent: usize) -> Result<Node<'a>> {
        let first = self.plain_line();
        let mut owned: Option<String> = None;
        self.finish_line()?;
        while let Some(ind) = self.peek_content_line()? {
            if ind <= parent_indent {
                break;
            }
            // Continuation line of a multi-line plain scalar (a key there is an error).
            self.pos += ind;
            if self.looks_like_key() {
                return self.err("unexpected mapping key (bad indentation?)");
            }
            let more = self.plain_line();
            let buf = owned.get_or_insert_with(|| first.to_string());
            buf.push(' ');
            buf.push_str(more);
            self.finish_line()?;
        }
        Ok(Node::Scalar(owned.map_or(Cow::Borrowed(first), Cow::Owned)))
    }

    /// Reads a plain scalar to end of line or ` #`, trimmed.
    fn plain_line(&mut self) -> &'a str {
        let start = self.pos;
        let end = self.line_end(start);
        let mut stop = end;
        let line = &self.b[start..end];
        let mut i = 0;
        while let Some(rel) = memchr::memchr(b'#', &line[i..]) {
            let at = i + rel;
            if at > 0 && matches!(line[at - 1], b' ' | b'\t') {
                stop = start + at;
                break;
            }
            i = at + 1;
        }
        self.pos = stop;
        self.src[start..stop].trim_end_matches([' ', '\t', '\r'])
    }

    fn block_scalar(&mut self, parent_indent: usize) -> Result<Node<'a>> {
        let folded = self.peek() == Some(b'>');
        let header_end = self.line_end(self.pos);
        let header = &self.src[self.pos..header_end];
        let keep = header.contains('+');
        let strip = header.contains('-');
        self.pos = (header_end + 1).min(self.b.len());
        let mut lines: Vec<&str> = Vec::new();
        let mut block_indent: Option<usize> = None;
        while self.pos < self.b.len() {
            let end = self.line_end(self.pos);
            let line = self.src[self.pos..end].trim_end_matches('\r');
            let ind = line.len() - line.trim_start_matches(' ').len();
            if line.trim().is_empty() {
                lines.push("");
                self.pos = (end + 1).min(self.b.len());
                continue;
            }
            if ind <= parent_indent {
                break;
            }
            let bi = *block_indent.get_or_insert(ind);
            if ind < bi {
                break;
            }
            lines.push(&line[bi..]);
            self.pos = (end + 1).min(self.b.len());
        }
        // Trailing blank lines belong to chomping, and the cursor must not skip past
        // content.
        let mut trailing = 0;
        while lines.last() == Some(&"") {
            lines.pop();
            trailing += 1;
        }
        let mut s = if folded {
            lines.join(" ")
        } else {
            lines.join("\n")
        };
        if !strip {
            s.push('\n');
            if keep {
                s.extend(std::iter::repeat_n('\n', trailing));
            }
        }
        Ok(Node::Scalar(Cow::Owned(s)))
    }

    fn skip_flow_ws(&mut self) -> Result<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'#') => self.pos = self.line_end(self.pos),
                None => return self.err("unterminated flow collection"),
                _ => return Ok(()),
            }
        }
    }

    fn flow_value(&mut self) -> Result<Node<'a>> {
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                let mut entries = Vec::new();
                loop {
                    self.skip_flow_ws()?;
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        return Ok(Node::Map(entries));
                    }
                    let key = match self.peek() {
                        Some(b'\'') => self.single_quoted()?,
                        Some(b'"') => self.double_quoted()?,
                        _ => self.flow_plain(true)?,
                    };
                    self.skip_flow_ws()?;
                    let value = if self.peek() == Some(b':') {
                        self.pos += 1;
                        self.skip_flow_ws()?;
                        if matches!(self.peek(), Some(b',' | b'}')) {
                            Node::Null
                        } else {
                            self.flow_item()?
                        }
                    } else {
                        Node::Null
                    };
                    entries.push((key, value));
                    self.skip_flow_ws()?;
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {}
                        _ => return self.err("expected ',' or '}' in flow mapping"),
                    }
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_flow_ws()?;
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        return Ok(Node::Seq(items));
                    }
                    items.push(self.flow_item()?);
                    self.skip_flow_ws()?;
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {}
                        _ => return self.err("expected ',' or ']' in flow sequence"),
                    }
                }
            }
            _ => self.err("expected flow collection"),
        }
    }

    fn flow_item(&mut self) -> Result<Node<'a>> {
        match self.peek() {
            Some(b'{' | b'[') => self.flow_value(),
            Some(b'\'') => Ok(Node::Scalar(self.single_quoted()?)),
            Some(b'"') => Ok(Node::Scalar(self.double_quoted()?)),
            _ => Ok(Node::Scalar(self.flow_plain(false)?)),
        }
    }

    /// A plain scalar in flow context: ends at `,`, `]`, `}`, or (for keys) `:
    /// `.
    fn flow_plain(&mut self, is_key: bool) -> Result<Cow<'a, str>> {
        let start = self.pos;
        loop {
            match self.peek() {
                None => return self.err("unterminated flow collection"),
                Some(b',' | b']' | b'}' | b'\n' | b'\r') => break,
                Some(b':')
                    if matches!(
                        self.b.get(self.pos + 1),
                        Some(b' ' | b',' | b'}' | b']' | b'\n' | b'\r')
                    ) && is_key =>
                {
                    break;
                }
                Some(b'#') if self.pos > start && self.b[self.pos - 1] == b' ' => break,
                _ => self.pos += 1,
            }
        }
        Ok(Cow::Borrowed(self.src[start..self.pos].trim()))
    }
}

/// YAML line folding for multi-line quoted scalars: a single line break becomes
/// a space.
fn fold_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut lines = s
        .split('\n')
        .map(|l| l.trim_matches([' ', '\t', '\r']))
        .peekable();
    let mut first = true;
    while let Some(l) = lines.next() {
        if l.is_empty() && !first && lines.peek().is_some() {
            out.push('\n');
            continue;
        }
        if !first && !out.ends_with('\n') && !l.is_empty() {
            out.push(' ');
        }
        out.push_str(l);
        first = false;
    }
    out
}

/// Parses a whole document into one tree (used by tests and small inputs).
#[cfg(test)]
pub(crate) fn parse_document(src: &str) -> Result<Node<'_>> {
    let mut p = Parser::new(src);
    let mut entries = Vec::new();
    p.top_level(|p, k| {
        let v = p.value(0)?;
        entries.push((k, v));
        Ok(())
    })?;
    Ok(Node::Map(entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Node<'_> {
        Node::Scalar(Cow::Borrowed(v))
    }

    // rustfmt's `format_strings` would change this item's string literals.
    #[rustfmt::skip]
    #[test]
    fn block_and_flow() {
        let doc = "\
# comment
lockfileVersion: '9.0'

settings:
  autoInstallPeers: true   # trailing comment
packages:

  '@a/b@1.0.0':
    resolution: {integrity: sha512-x==, tarball: https://r.example/a.tgz}
    cpu: [arm64, x64]
    engines: {node: '>=18'}
    deps:
      - encoding
      - 'quoted # not comment'
  plain@2.0.0(peer@1.0.0): {}
";
        let n = parse_document(doc).unwrap();
        assert_eq!(n.get_str("lockfileVersion"), Some("9.0"));
        assert_eq!(
            n.get("settings").unwrap().get_str("autoInstallPeers"),
            Some("true")
        );
        let pkgs = n.get("packages").unwrap();
        let ab = pkgs.get("@a/b@1.0.0").unwrap();
        let res = ab.get("resolution").unwrap();
        assert_eq!(res.get_str("integrity"), Some("sha512-x=="));
        assert_eq!(res.get_str("tarball"), Some("https://r.example/a.tgz"));
        assert_eq!(ab.get("cpu").unwrap().str_list(), vec!["arm64", "x64"]);
        assert_eq!(ab.get("engines").unwrap().get_str("node"), Some(">=18"));
        assert_eq!(
            ab.get("deps").unwrap(),
            &Node::Seq(vec![s("encoding"), s("quoted # not comment")])
        );
        assert_eq!(
            pkgs.get("plain@2.0.0(peer@1.0.0)").unwrap(),
            &Node::Map(vec![])
        );
    }

    #[test]
    fn quoting_and_escapes() {
        let doc = "a: 'it''s'\nb: \"x\\ty\\u00e9\\\"\"\n\"c d\": plain value\n'e:f': \"\"\n";
        let n = parse_document(doc).unwrap();
        assert_eq!(n.get_str("a"), Some("it's"));
        assert_eq!(n.get_str("b"), Some("x\tyé\""));
        assert_eq!(n.get_str("c d"), Some("plain value"));
        assert_eq!(n.get_str("e:f"), Some(""));
    }

    #[test]
    fn crlf_block_scalars_and_compact_seq() {
        let doc = "a:\r\n  b: 1\r\n  msg: |-\r\n    line one\r\n    line two\r\n  list:\r\n  - \
                   name: x\r\n    v: 1\r\n  - y\r\nz: 2\r\n";
        let n = parse_document(doc).unwrap();
        let a = n.get("a").unwrap();
        assert_eq!(a.get_str("b"), Some("1"));
        assert_eq!(a.get_str("msg"), Some("line one\nline two"));
        let list = a.get("list").unwrap();
        assert_eq!(
            list,
            &Node::Seq(vec![
                Node::Map(vec![
                    (Cow::Borrowed("name"), s("x")),
                    (Cow::Borrowed("v"), s("1"))
                ]),
                s("y")
            ])
        );
        assert_eq!(n.get_str("z"), Some("2"));
    }

    #[test]
    fn multiline_flow_and_plain_continuation() {
        let doc = "a: {x: 1,\n  y: [p,\n   q]}\nlong: one\n  two\nb: c\n";
        let n = parse_document(doc).unwrap();
        assert_eq!(
            n.get("a").unwrap().get("y").unwrap().str_list(),
            vec!["p", "q"]
        );
        assert_eq!(n.get_str("long"), Some("one two"));
        assert_eq!(n.get_str("b"), Some("c"));
    }

    #[test]
    fn documents() {
        let src = "---\na: 1\n---\nb: 2\n";
        assert_eq!(Parser::document_count(src), 2);
        let mut p = Parser::new(src);
        p.seek_document(1).unwrap();
        let mut keys = Vec::new();
        p.top_level(|p, k| {
            keys.push(k.to_string());
            p.value(0).map(drop)
        })
        .unwrap();
        assert_eq!(keys, vec!["b"]);
        assert_eq!(Parser::document_count("a: 1\n"), 1);
    }

    /// js-yaml (pnpm) writes a key longer than 1024 characters in the explicit
    /// form.
    #[test]
    fn explicit_keys() {
        let doc = "\
snapshots:

  a@1.0.0: {}

  ? 'long@1.0.0(peer@1.0.0)'
  : dependencies:
      x: 1.0.0
      y: 2.0.0(z@1.0.0)
    optionalDependencies:
      w: 3.0.0

  ? plain@2.0.0(peer@1.0.0)
  : {}

  ? scalar
  : value
  ? below
  :
    k: v
  ? none
  z@1.0.0:
    dependencies:
      q: 1.0.0
after: 1
";
        let n = parse_document(doc).unwrap();
        let snaps = n.get("snapshots").unwrap();
        let keys: Vec<&str> = snaps.entries().iter().map(|(k, _)| k.as_ref()).collect();
        assert_eq!(
            keys,
            [
                "a@1.0.0",
                "long@1.0.0(peer@1.0.0)",
                "plain@2.0.0(peer@1.0.0)",
                "scalar",
                "below",
                "none",
                "z@1.0.0"
            ]
        );
        let long = snaps.get("long@1.0.0(peer@1.0.0)").unwrap();
        let deps = long.get("dependencies").unwrap();
        assert_eq!(deps.get_str("x"), Some("1.0.0"));
        assert_eq!(deps.get_str("y"), Some("2.0.0(z@1.0.0)"));
        assert_eq!(
            long.get("optionalDependencies").unwrap().get_str("w"),
            Some("3.0.0")
        );
        assert_eq!(
            snaps.get("plain@2.0.0(peer@1.0.0)").unwrap(),
            &Node::Map(vec![])
        );
        assert_eq!(snaps.get_str("scalar"), Some("value"));
        assert_eq!(snaps.get("below").unwrap().get_str("k"), Some("v"));
        assert_eq!(snaps.get("none").unwrap(), &Node::Null);
        assert_eq!(
            snaps
                .get("z@1.0.0")
                .unwrap()
                .get("dependencies")
                .unwrap()
                .get_str("q"),
            Some("1.0.0")
        );
        assert_eq!(n.get_str("after"), Some("1"));
        // The streaming reader sees the same entries.
        let mut p = Parser::new(doc);
        let mut seen = Vec::new();
        p.top_level(|p, k| {
            if k == "snapshots" {
                p.entries(0, |k, v| {
                    seen.push((k.to_string(), v.entries().len()));
                    Ok(())
                })
            } else {
                p.value(0).map(drop)
            }
        })
        .unwrap();
        assert_eq!(seen.len(), 7);
        assert_eq!(seen[1], ("long@1.0.0(peer@1.0.0)".to_string(), 2));
        // A key that is a collection is rejected, with its line.
        let e = parse_document("a:\n  ? [x, y]\n  : 1\n").unwrap_err();
        assert_eq!(e.line, 2);
        // `?name` without a space is a plain key.
        assert_eq!(parse_document("?a: 1\n").unwrap().get_str("?a"), Some("1"));
    }

    #[test]
    fn errors_have_lines() {
        let e = parse_document("a: 1\n   b: 2\n").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(parse_document("a: &x 1\n").is_err());
        assert!(parse_document("a: 'open\n").is_err());
    }
}
