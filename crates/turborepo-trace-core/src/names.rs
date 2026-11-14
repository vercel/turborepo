//! Path names: the strings of a file that look like a file name or path, as a
//! per-file fact any language plugin can emit.
//!
//! A read whose path cannot be evaluated (a parameter, the result of a call, a
//! value built by code the analysis does not follow) still reads a path that
//! was assembled from text. Like a conservative garbage collector, which treats
//! every word that could be a pointer as one, a change query treats every
//! string that could be (part of) a path as a possible name of the repository
//! files of that shape (`turborepo_trace_graph::names`). This module is
//! the language-neutral half of collecting them: what a name is ([`shape_of`],
//! defined once for every language), what a file's names are ([`Names`]), and
//! how they travel with the file's extraction as a fact ([`Names::encode`],
//! [`RefSink::fact`](crate::RefSink::fact), [`decode`]).
//!
//! # What a plugin does
//!
//! Walk the string literals of the file (not import specifiers: those are
//! references already) and give each to a [`Collector`]: [`Collector::string`]
//! with the decoded text, whether the literal is an argument of a path function
//! of the language (`path.join`, `filepath.Join`, `os.Open`), and its line. Set
//! [`Names::anchor`], [`Names::cwd`] and [`Names::test_path`] for the
//! language's ways to name the file's own location, the working directory and
//! the running test, and emit `sink.fact(names.encode())` when
//! [`Names::is_empty`] is false. A language whose records should then be
//! bounded by names is listed by the caller of
//! `turborepo_trace_graph::AffectedOptions::named`.
//!
//! # What is a name
//!
//! A string is first split into words at whitespace and at characters that
//! separate values rather than belong to a file name (`=`, `,`, `;`, `|`, `&`,
//! quotes), so a path inside a command line or a message is found
//! (`'--config=configs/a.json'`); a word is cut at `?` or `#`. Words with a `:`
//! (URLs, `node:fs`, route parameters) are not names. Each word is reduced to a
//! *shape*, a run of literal path segments:
//!
//! - a leading `./`, `../` or `/` and a trailing `/` are dropped (the base is
//!   unknown);
//! - a hole (a template substitution, written as [`HOLE`]; a glob wildcard; a
//!   brace group) drops the leading segments that have one and ends the shape
//!   before the next segment with one; what follows is kept as the *rest*
//!   ([`Name::rest`]), a pattern the path below the shape has to fit:
//!   `fixtures/${name}.json` names the `.json` files below a directory
//!   `fixtures`, `/docs/configuration${x}` names what starts with
//!   `configuration` in a directory `docs`, `/services/${id}/apply` needs an
//!   `apply` below `services`. A hole stands for any text, with separators or
//!   empty;
//! - a `.` or `..` segment inside restarts the shape after it;
//! - `@scope/name/rest` names `rest` (the package directory is not known here).
//!
//! The shape is a name when one of these holds ([`NameKind`]):
//!
//! | kind | condition | example |
//! |---|---|---|
//! | `Path` | at least two segments, or one segment written with path syntax (a separator, a hole next to a separator) | `fixtures/a.json`, `./data`, `/snapshots/${id}` |
//! | `File` | one segment with an extension | `schema.sql` |
//! | `Word` | one plain segment that is an argument of a path function | `join(dir, 'fixtures', name)` |
//!
//! A bare word anywhere else is not a name: `'user'`, `'api'` and `'test'` are
//! directory names in most repositories, and treating them as names selects
//! everything.
//!
//! # The fact
//!
//! `names:<anchor>,<cwd>,<test_path>,<ups>,<overflow>` followed by one
//! `\n<line>\t<shape>` per name, or `\n<line>\t<shape>\t<rest>` for a name with
//! a rest; lines are 1-based, `0` is "none". Shapes and rests contain no
//! control characters. When the plugin knows where the strings are, `<line>` is
//! `<line>@<first>` or `<line>@<first>-<last>`: the byte offsets of the first
//! and the last string with that name ([`line_and_span`]), so that a
//! consumer can leave out the names inside a region of the file (a table whose
//! values an analysis resolved). A `^` at the end of the field marks a name
//! whose string climbs ([`Name::climbs`], [`climbs`]).
//!
//! A rest is a list of elements joined with `/`: a literal segment; `text*`, a
//! segment that starts with the text, followed by any number of segments;
//! `*text`, any number of segments and one that ends with the text; `*`, any
//! number of segments.
//!
//! # Anchored names
//!
//! A base that is not a string (the file's own location, the working directory)
//! names everything below it: [`Names::anchor`], [`Names::cwd`]. A plugin that
//! follows where such a base goes replaces that with [`Anchored`] entries, one
//! per place the base (or a path built on it) leaves what the plugin evaluates:
//! the directory it had there, relative to the base, and what is read in it
//! (everything at or below, or the files directly in it that match a pattern,
//! for a directory handed to a function known to read only those).
//! [`Names::anchor`] and [`Names::cwd`] are then `None`: the entries say all
//! there is. In the fact an entry is a line
//! `\n@<L|C><?>\t<line>\t<rel>\t<pattern>`: `L` for the file's directory, `C`
//! for the working directory; `?` marks an entry that only counts
//! while the file has a counted record at that line
//! ([`Anchored::conditional`]); `<rel>` is a normalized relative path (`` for
//! the base itself, `../..`, `../fixtures`); `<pattern>` is `**` or a file-name
//! glob with `*`.

use std::collections::{HashMap, hash_map::Entry};

/// The tag of a names fact.
pub const TAG: &str = "names:";
/// Maximum length of a word considered.
pub const MAX_LEN: usize = 300;
/// Maximum length of a string or template piece considered.
pub const MAX_STRING: usize = 8192;
/// Maximum value of [`Names::ups`].
pub const MAX_UPS: u32 = 64;
/// Maximum names per file. A file with more names everything
/// ([`Names::overflow`]).
pub const MAX_NAMES: usize = 16_384;
/// The hole marker inside a string given to [`shape_of`]: a part that is not
/// literal text.
pub const HOLE: char = '\u{1}';

/// Why a shape is a name.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NameKind {
    /// Two or more segments, or one segment written with path syntax.
    Path,
    /// One segment with an extension.
    File,
    /// One plain segment in an argument of a path function.
    Word,
}

/// One name of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// Literal segments joined with `/`; never empty, no leading or trailing
    /// `/`.
    pub shape: Box<str>,
    /// What the path below the shape has to fit (see the module documentation);
    /// `None`: anything.
    pub rest: Option<Box<str>>,
    /// Why it is a name.
    pub kind: NameKind,
    /// 1-based line of the string (its first occurrence).
    pub line: u32,
    /// Byte offsets of the first and the last string with this name, when
    /// known.
    pub span: Option<(u32, u32)>,
    /// A string with this name was written with parent steps that lead out of
    /// the directory it starts from (`../fixtures/a.json`, `join(base,
    /// '..', 'data')`): the code says that the path leaves. A consumer that
    /// assumes a path stays inside some directory (the package assumption
    /// of `turborepo_trace_graph::names`) does not hold such a name to
    /// it. Not set for a literal path from the file's own location: where that
    /// goes is an [`Anchored`] entry.
    pub climbs: bool,
}

/// One place where a base that is not a string leaves what the plugin evaluates
/// (see the module documentation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchored {
    /// The base is the working directory; otherwise the directory of the file.
    pub cwd: bool,
    /// 1-based line.
    pub line: u32,
    /// The directory, relative to the base: normalized, `""` for the base
    /// itself, leading `..` segments for directories above it.
    pub rel: Box<str>,
    /// What is read there: `**` for everything at or below the directory,
    /// otherwise a file-name pattern (literal text and `*`) for files
    /// directly in it.
    pub pattern: Box<str>,
    /// The entry counts only while the file has a counted uncertainty record at
    /// [`Anchored::line`]: the path continues with text another layer owns (the
    /// environment), and that layer's verdict on the record at the same place
    /// decides.
    pub conditional: bool,
}

/// The pattern of an [`Anchored`] entry that names everything at or below its
/// directory.
pub const EVERYTHING_BELOW: &str = "**";

/// The names of one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    /// Distinct shapes, in source order.
    pub names: Vec<Name>,
    /// Line of the first use of the file's own location (`__dirname`,
    /// `import.meta.url`) that is not a literal path below it.
    pub anchor: Option<u32>,
    /// Line of the first use of the working directory (`cwd()`, `INIT_CWD`,
    /// `PWD`).
    pub cwd: Option<u32>,
    /// Line of the first use of the runner's current test file (`testPath`).
    pub test_path: Option<u32>,
    /// Number of `..` segments in strings plus `dirname` calls (at most
    /// [`MAX_UPS`]), outside literal paths below the file's location: how
    /// far above its own directory or the working directory the file can
    /// reach.
    pub ups: u32,
    /// More than [`MAX_NAMES`] names: the file is taken to name every path.
    pub overflow: bool,
    /// Where the file's location and the working directory go, when the plugin
    /// followed every use of them ([`Anchored`]).
    pub anchored: Vec<Anchored>,
}

impl Names {
    /// Whether the file has no name and no anchor.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
            && self.anchored.is_empty()
            && self.anchor.is_none()
            && self.cwd.is_none()
            && self.test_path.is_none()
            && !self.overflow
    }

    /// The fact for [`RefSink::fact`](crate::RefSink::fact).
    pub fn encode(&self) -> String {
        let mut out = String::with_capacity(TAG.len() + 16 + self.names.len() * 24);
        out.push_str(TAG);
        out.push_str(&format!(
            "{},{},{},{},{}",
            self.anchor.unwrap_or(0),
            self.cwd.unwrap_or(0),
            self.test_path.unwrap_or(0),
            self.ups,
            u8::from(self.overflow)
        ));
        for n in &self.names {
            out.push('\n');
            out.push_str(&n.line.to_string());
            if let Some((first, last)) = n.span {
                out.push('@');
                out.push_str(&first.to_string());
                if last != first {
                    out.push('-');
                    out.push_str(&last.to_string());
                }
            }
            if n.climbs {
                out.push('^');
            }
            out.push('\t');
            out.push_str(&n.shape);
            if let Some(rest) = &n.rest {
                out.push('\t');
                out.push_str(rest);
            }
        }
        for a in &self.anchored {
            out.push_str(&format!(
                "\n@{}{}\t{}\t{}\t{}",
                if a.cwd { 'C' } else { 'L' },
                if a.conditional { "?" } else { "" },
                a.line,
                a.rel,
                a.pattern
            ));
        }
        out
    }
}

/// A names fact, read in place.
#[derive(Copy, Clone, Debug)]
pub struct Encoded<'a> {
    /// See [`Names::anchor`].
    pub anchor: Option<u32>,
    /// See [`Names::cwd`].
    pub cwd: Option<u32>,
    /// See [`Names::test_path`].
    pub test_path: Option<u32>,
    /// See [`Names::ups`].
    pub ups: u32,
    /// See [`Names::overflow`].
    pub overflow: bool,
    body: &'a str,
}

impl<'a> Encoded<'a> {
    /// The names: `(line, shape, rest)`.
    pub fn names(&self) -> impl Iterator<Item = (u32, &'a str, Option<&'a str>)> + 'a {
        self.shapes()
            .map(|(line, shape, rest)| (line_and_span(line).0, shape, rest))
    }

    /// The names without parsing the lines: `(line as text, shape, rest)`.
    pub fn shapes(&self) -> impl Iterator<Item = (&'a str, &'a str, Option<&'a str>)> + 'a {
        self.body.split('\n').filter_map(|l| {
            if l.starts_with('@') {
                return None;
            }
            let (line, shape) = l.split_once('\t')?;
            Some(match shape.split_once('\t') {
                Some((shape, rest)) => (line, shape, Some(rest)),
                None => (line, shape, None),
            })
        })
    }

    /// The anchored entries ([`Names::anchored`]).
    pub fn anchored(&self) -> impl Iterator<Item = Anchored> + 'a {
        self.body.split('\n').filter_map(|l| {
            let mut it = l.strip_prefix('@')?.split('\t');
            let head = it.next()?;
            let line = it.next()?.parse().ok()?;
            let rel = it.next()?;
            let pattern = it.next()?;
            Some(Anchored {
                cwd: head.starts_with('C'),
                line,
                rel: rel.into(),
                pattern: pattern.into(),
                conditional: head.ends_with('?'),
            })
        })
    }

    /// Whether there is nothing in it.
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
            && self.anchor.is_none()
            && self.cwd.is_none()
            && self.test_path.is_none()
            && !self.overflow
    }
}

/// Whether the first field of a name marks it as climbing ([`Name::climbs`]).
pub fn climbs(field: &str) -> bool {
    field.ends_with('^')
}

/// Whether the path `word` leads out of the directory it starts from: a `..`
/// segment with no named segment before it left to remove (`../a`, `a/../../b`;
/// not `a/../b`). A hole counts as a segment.
pub fn climbs_out(word: &str) -> bool {
    let mut depth = 0u32;
    for s in word.split('/') {
        match s {
            "" | "." => {}
            ".." if depth == 0 => return true,
            ".." => depth -= 1,
            _ => depth += 1,
        }
    }
    false
}

/// The line and the offsets of the first and last occurrence in the first field
/// of a name (`<line>`, `<line>@<first>`, `<line>@<first>-<last>`, each with an
/// optional `^`).
pub fn line_and_span(field: &str) -> (u32, Option<(u32, u32)>) {
    let field = field.strip_suffix('^').unwrap_or(field);
    let Some((line, span)) = field.split_once('@') else {
        return (field.parse().unwrap_or(0), None);
    };
    let (first, last) = span.split_once('-').unwrap_or((span, span));
    let span = match (first.parse(), last.parse()) {
        (Ok(a), Ok(b)) => Some((a, b)),
        _ => None,
    };
    (line.parse().unwrap_or(0), span)
}

/// Reads a fact; `None` when it is not a names fact.
pub fn decode(fact: &str) -> Option<Encoded<'_>> {
    let rest = fact.strip_prefix(TAG)?;
    let (head, body) = rest.split_once('\n').unwrap_or((rest, ""));
    let mut it = head.split(',').map(|x| x.parse::<u32>().unwrap_or(0));
    let line = |x: u32| (x > 0).then_some(x);
    Some(Encoded {
        anchor: line(it.next()?),
        cwd: line(it.next()?),
        test_path: line(it.next()?),
        ups: it.next()?,
        overflow: it.next()? != 0,
        body,
    })
}

/// Characters that separate words inside a string.
pub fn separates(c: char) -> bool {
    (c <= ' ' && c != HOLE)
        || c == '\u{7f}'
        || matches!(
            c,
            '=' | ',' | ';' | '|' | '&' | '"' | '\'' | '`' | '\\' | '<' | '>' | '^' | '!' | '%'
        )
}

/// The words of a string that can be names: split at [`separates`], cut at `?`
/// and `#`, without the words that have a `:`.
pub fn words(value: &str) -> impl Iterator<Item = &str> {
    value.split(separates).filter_map(|w| {
        let w = w.split(['?', '#']).next().unwrap_or("");
        (!w.is_empty() && w.len() <= MAX_LEN && !w.contains(':') && !w.starts_with("//"))
            .then_some(w)
    })
}

/// Number of `..` segments of a word.
pub fn ups_of(word: &str) -> u32 {
    word.split('/').filter(|s| *s == "..").count() as u32
}

/// Whether a word has a segment that is not `.` or `..`.
pub fn goes_down(word: &str) -> bool {
    word.split('/').any(|s| !matches!(s, "" | "." | ".."))
}

fn literal_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    b'_' | b'.' | b'@' | b'+' | b'~' | b'$' | b'(' | b')' | b'[' | b']' | b'-'
                )
        })
        && s.bytes().any(|c| c.is_ascii_alphanumeric())
}

/// Whether `s` is `stem.ext`: a name with an extension of letters and digits
/// that is not a number (`1.5`, `v1.2`).
fn has_extension(s: &str) -> bool {
    let Some((stem, ext)) = s.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !ext.is_empty()
        && ext.len() <= 10
        && ext.bytes().all(|c| c.is_ascii_alphanumeric())
        && ext.bytes().any(|c| c.is_ascii_alphabetic())
        && stem.bytes().any(|c| c.is_ascii_alphabetic())
}

/// The shape and kind of the word `value` (holes are [`HOLE`]); `path_arg`: it
/// is an argument of a path function. `None`: not a name.
pub fn shape_of(value: &str, path_arg: bool) -> Option<(String, NameKind)> {
    pattern_of(value, path_arg).map(|(shape, _, kind)| (shape, kind))
}

/// One element of a rest for the segment `seg` of a word, and whether the rest
/// goes on after it. `None`: the segment says nothing that can be checked (the
/// rest ends before it).
fn rest_element(seg: &str) -> Option<(String, bool)> {
    // A hole of any kind is `*`; a brace group and everything after it in the
    // segment too.
    let mut text = String::with_capacity(seg.len());
    for c in seg.chars() {
        match c {
            '{' | '}' => {
                text.push('*');
                break;
            }
            HOLE | '*' => {
                if !text.ends_with('*') {
                    text.push('*');
                }
            }
            c => text.push(c),
        }
    }
    let literal = |t: &str| t.is_empty() || literal_segment(t);
    match text.find('*') {
        None => literal_segment(&text).then_some((text, true)),
        Some(_) if text == "*" => Some((text, true)),
        // `*text`: the hole may hold separators, so any segments may come first.
        Some(0) if !text[1..].contains('*') => literal(&text[1..]).then_some((text, true)),
        // `text*`, or more holes: only the start of the segment is checked.
        Some(i) if i > 0 && literal(&text[..i]) => {
            let whole = i + 1 == text.len();
            Some((format!("{}*", &text[..i]), whole))
        }
        Some(_) => None,
    }
}

/// The shape, rest and kind of the word `value` (holes are [`HOLE`]);
/// `path_arg`: it is an argument of a path function. `None`: not a name.
pub fn pattern_of(value: &str, path_arg: bool) -> Option<(String, Option<String>, NameKind)> {
    if value.is_empty() || value.len() > MAX_LEN || value.contains(':') || value.starts_with("//") {
        return None;
    }
    if value.chars().any(separates) || value.contains(['?', '#']) {
        return None;
    }
    // Path syntax: a separator anywhere.
    let syntax = value.contains('/');
    let mut segs: Vec<&str> = value.split('/').collect();
    // `@scope/name/rest`: the package's directory is not known here.
    if segs.len() >= 2 && segs[0].starts_with('@') && segs[0].len() > 1 {
        segs.drain(..2);
    }
    // The run of literal segments after the last `.`/`..` and after leading holes.
    let is_hole =
        |s: &str| s.contains(HOLE) || s.contains('*') || s.contains('{') || s.contains('}');
    let mut run: Vec<&str> = Vec::new();
    let mut closed = false;
    // The elements after the run, while they can be checked.
    let mut rest: Vec<String> = Vec::new();
    let mut rest_open = true;
    for s in segs {
        match s {
            "" => {}
            "." | ".." => {
                run.clear();
                closed = false;
                rest.clear();
                rest_open = true;
            }
            s if is_hole(s) => {
                // Leading holes are dropped; a later hole ends the run.
                if !run.is_empty() {
                    closed = true;
                    if rest_open {
                        match rest_element(s) {
                            Some((e, more)) => {
                                rest.push(e);
                                rest_open = more;
                            }
                            None => rest_open = false,
                        }
                    }
                }
            }
            s => {
                if closed {
                    if rest_open {
                        match rest_element(s) {
                            Some((e, more)) => {
                                rest.push(e);
                                rest_open = more;
                            }
                            None => rest_open = false,
                        }
                    }
                    continue;
                }
                if !literal_segment(s) {
                    return None;
                }
                run.push(s);
            }
        }
    }
    let kind = match run.len() {
        0 => return None,
        1 if syntax => NameKind::Path,
        1 if has_extension(run[0]) => NameKind::File,
        1 if path_arg => NameKind::Word,
        1 => return None,
        _ => NameKind::Path,
    };
    // A rest that ends with "anything" says no more than one without it.
    while rest.last().is_some_and(|e| e == "*") {
        rest.pop();
    }
    let rest = (!rest.is_empty()).then(|| rest.join("/"));
    Some((run.join("/"), rest, kind))
}

/// Collects the names of one file.
#[derive(Debug, Default)]
pub struct Collector {
    /// The names so far.
    pub names: Names,
    /// Byte offset of the string the next words come from, when the plugin
    /// knows it.
    pub at: Option<u32>,
    seen: HashMap<(String, Option<String>, NameKind), usize>,
}

impl Collector {
    /// An empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the name of `word` (one word of a string, see [`words`]), if it is
    /// one and new, and marks it as climbing when the word leads out of the
    /// directory it starts from ([`climbs_out`]).
    pub fn word(&mut self, word: &str, path_arg: bool, line: u32) {
        if let Some(i) = self.word_at(word, path_arg, line)
            && climbs_out(word)
        {
            self.mark_climbs(i);
        }
    }

    /// As [`Collector::word`], without the climbing mark: the index of the name
    /// in [`Names::names`], for a plugin that decides later whether the
    /// parent steps of the word count ([`Collector::mark_climbs`]). `None`:
    /// not a name, or over the cap.
    pub fn word_at(&mut self, word: &str, path_arg: bool, line: u32) -> Option<usize> {
        let (shape, rest, kind) = pattern_of(word, path_arg)?;
        let next = self.names.names.len();
        match self.seen.entry((shape.clone(), rest.clone(), kind)) {
            Entry::Occupied(e) => {
                // Seen before: this is its last occurrence so far.
                if let (Some(n), Some(at)) = (self.names.names.get_mut(*e.get()), self.at) {
                    n.span = Some((n.span.map_or(at, |s| s.0), at));
                }
                Some(*e.get())
            }
            Entry::Vacant(_) if next >= MAX_NAMES => {
                self.names.overflow = true;
                None
            }
            Entry::Vacant(e) => {
                e.insert(next);
                self.names.names.push(Name {
                    shape: shape.into(),
                    rest: rest.map(Into::into),
                    kind,
                    line,
                    span: self.at.map(|at| (at, at)),
                    climbs: false,
                });
                Some(next)
            }
        }
    }

    /// Marks the name at index `i` ([`Collector::word_at`]) as climbing
    /// ([`Name::climbs`]).
    pub fn mark_climbs(&mut self, i: usize) {
        if let Some(n) = self.names.names.get_mut(i) {
            n.climbs = true;
        }
    }

    /// Adds the names of the string `value` at `line` and counts its `..`
    /// segments. `path_arg`: the string is an argument of a path function
    /// (it counts for a string that is one word).
    pub fn string(&mut self, value: &str, path_arg: bool, line: u32) {
        if value.len() > MAX_STRING {
            return;
        }
        let whole = !value.chars().any(separates);
        for w in words(value) {
            self.names.ups = (self.names.ups + ups_of(w)).min(MAX_UPS);
            self.word(w, path_arg && whole, line);
        }
    }

    /// The names collected.
    pub fn finish(self) -> Names {
        self.names
    }
}

/// The names in the text of a data file that is not JSON (YAML, TOML, plain
/// text): every word of every line, with quotes, brackets and trailing `:`
/// removed.
pub fn scan_text(src: &[u8]) -> Names {
    let mut c = Collector::new();
    for (i, line) in src.split(|&b| b == b'\n').enumerate() {
        if line.len() > MAX_STRING {
            continue;
        }
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        for word in line.split(|c: char| separates(c) || matches!(c, '[' | ']' | '{' | '}')) {
            c.string(word.trim_end_matches(':'), false, i as u32 + 1);
        }
    }
    c.finish()
}

/// The names in a JSON file: its strings (keys and values).
pub fn scan_json(src: &[u8]) -> Names {
    let mut c = Collector::new();
    let mut line = 1u32;
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'"' => {
                let start_line = line;
                let mut text: Vec<u8> = Vec::new();
                i += 1;
                while i < src.len() && src[i] != b'"' {
                    match src[i] {
                        b'\\' => {
                            // `\/` and `\\` are themselves; any other escape separates words.
                            match src.get(i + 1) {
                                Some(&(b'/' | b'\\')) => text.push(src[i + 1]),
                                _ => text.push(b' '),
                            }
                            i += 2;
                            continue;
                        }
                        b'\n' => line += 1,
                        _ => {}
                    }
                    text.push(src[i]);
                    i += 1;
                }
                i += 1;
                if let Ok(s) = std::str::from_utf8(&text) {
                    c.string(s, false, start_line);
                }
            }
            _ => i += 1,
        }
    }
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_round_trip() {
        let mut c = Collector::new();
        c.string("fixtures/a.json", false, 3);
        c.string("node scripts/build.js --out=dist/app ../x", false, 7);
        let mut n = c.finish();
        n.anchor = Some(2);
        n.test_path = Some(9);
        let fact = n.encode();
        assert_eq!(
            fact,
            "names:2,0,9,1,0\n3\tfixtures/a.json\n7\tscripts/build.js\n7\tdist/app\n7^\tx"
        );
        let d = decode(&fact).unwrap();
        assert_eq!(
            (d.anchor, d.cwd, d.test_path, d.ups, d.overflow),
            (Some(2), None, Some(9), 1, false)
        );
        assert_eq!(
            d.names().collect::<Vec<_>>(),
            [
                (3, "fixtures/a.json", None),
                (7, "scripts/build.js", None),
                (7, "dist/app", None),
                (7, "x", None)
            ]
        );
        let mut c = Collector::new();
        c.string("/docs/configuration\u{1}", false, 1);
        c.string("docs/", false, 2);
        let fact = c.finish().encode();
        assert_eq!(fact, "names:0,0,0,0,0\n1\tdocs\tconfiguration*\n2\tdocs");
        assert_eq!(
            decode(&fact).unwrap().names().collect::<Vec<_>>(),
            [(1, "docs", Some("configuration*")), (2, "docs", None)]
        );
        // With offsets: the first and the last string of each name.
        let mut c = Collector::new();
        for (at, line, text) in [(10, 1, "a/b.json"), (40, 2, "c/d"), (90, 5, "a/b.json")] {
            c.at = Some(at);
            c.string(text, false, line);
        }
        let fact = c.finish().encode();
        assert_eq!(fact, "names:0,0,0,0,0\n1@10-90\ta/b.json\n2@40\tc/d");
        let d = decode(&fact).unwrap();
        assert_eq!(
            d.shapes()
                .map(|(l, s, _)| (line_and_span(l), s))
                .collect::<Vec<_>>(),
            [
                ((1, Some((10, 90))), "a/b.json"),
                ((2, Some((40, 40))), "c/d")
            ]
        );
        assert_eq!(d.names().map(|n| n.0).collect::<Vec<_>>(), [1, 2]);
        // A name whose string climbs out of where it starts is marked, with or without
        // offsets, and the mark is not part of the line.
        let mut c = Collector::new();
        c.string("../shared/a.json", false, 4);
        c.at = Some(70);
        c.string("x/../../y/b.json", false, 6);
        c.string("x/../y/c.json", false, 8);
        let fact = c.finish().encode();
        assert_eq!(
            fact,
            "names:0,0,0,4,0\n4^\tshared/a.json\n6@70^\ty/b.json\n8@70\ty/c.json"
        );
        let d = decode(&fact).unwrap();
        assert_eq!(
            d.shapes()
                .map(|(l, s, _)| (line_and_span(l), climbs(l), s))
                .collect::<Vec<_>>(),
            [
                ((4, None), true, "shared/a.json"),
                ((6, Some((70, 70))), true, "y/b.json"),
                ((8, Some((70, 70))), false, "y/c.json")
            ]
        );
        assert!(climbs_out("../a") && climbs_out("a/../../b") && climbs_out(".."));
        assert!(!climbs_out("a/../b") && !climbs_out("./a/b") && !climbs_out("/a"));
        assert!(decode("env:{}").is_none());
        let none = Names::default().encode();
        let empty = decode(&none).unwrap();
        assert!(empty.is_empty() && empty.names().next().is_none());
    }

    #[test]
    fn kinds() {
        assert_eq!(
            shape_of("fixtures/a.json", false),
            Some(("fixtures/a.json".into(), NameKind::Path))
        );
        assert_eq!(
            shape_of("./data", false),
            Some(("data".into(), NameKind::Path))
        );
        assert_eq!(
            shape_of("schema.sql", false),
            Some(("schema.sql".into(), NameKind::File))
        );
        assert_eq!(
            shape_of("fixtures", true),
            Some(("fixtures".into(), NameKind::Word))
        );
        assert_eq!(shape_of("fixtures", false), None);
        assert_eq!(shape_of("a:b/c", false), None);
        assert_eq!(shape_of("1.5", false), None);
    }

    #[test]
    fn rests() {
        let rest = |v: &str| pattern_of(v, false).map(|(s, r, _)| (s, r));
        let some = |s: &str, r: &str| Some((s.to_owned(), Some(r.to_owned())));
        let none = |s: &str| Some((s.to_owned(), None));
        // What follows the first hole is kept as long as it can be checked.
        assert_eq!(rest("fixtures/\u{1}.json"), some("fixtures", "*.json"));
        assert_eq!(
            rest("/docs/configuration\u{1}"),
            some("docs", "configuration*")
        );
        assert_eq!(rest("/services/\u{1}/apply"), some("services", "*/apply"));
        assert_eq!(
            rest("\u{1}/v2/providers/\u{1}/services/\u{1}/apply"),
            some("v2/providers", "*/services/*/apply")
        );
        assert_eq!(rest("src/**/*.test.ts"), some("src", "*/*.test.ts"));
        // A brace group is not checked.
        assert_eq!(rest("data/*.{json"), none("data"));
        assert_eq!(rest("a/pre-\u{1}-post/b"), some("a", "pre-*"));
        // A hole at the end, or only holes, is no rest.
        assert_eq!(rest("fixtures/\u{1}"), none("fixtures"));
        assert_eq!(rest("fixtures/\u{1}/\u{1}"), none("fixtures"));
        // `..` after a hole: the path leaves what was spelled; a new shape starts.
        assert_eq!(rest("a/\u{1}/../b/c"), none("b/c"));
        // No hole, no rest.
        assert_eq!(rest("fixtures/a.json"), none("fixtures/a.json"));
    }

    #[test]
    fn data_files() {
        let n = scan_json(
            br#"{"routes": [{"module": "svc/handlers/get", "n": 1}],
 "main": "dist\/index.js", "note": "see docs/guide.md\nand more"}"#,
        );
        let got: Vec<(&str, u32)> = n.names.iter().map(|n| (&*n.shape, n.line)).collect();
        assert_eq!(
            got,
            [
                ("svc/handlers/get", 1),
                ("dist/index.js", 2),
                ("docs/guide.md", 2)
            ]
        );
        let n = scan_text(
            b"seed:\n  file: db/seed.sql\n  dirs: [fixtures/a, 'fixtures/b']\n# see ../docs/x.md\nname: plain\n",
        );
        let got: Vec<(&str, u32)> = n.names.iter().map(|n| (&*n.shape, n.line)).collect();
        assert_eq!(
            got,
            [
                ("db/seed.sql", 2),
                ("fixtures/a", 3),
                ("fixtures/b", 3),
                ("docs/x.md", 4)
            ]
        );
        assert_eq!(n.ups, 1);
    }
    #[test]
    fn anchored_entries_round_trip() {
        let mut c = Collector::new();
        c.string("fixtures/a.json", false, 3);
        let mut n = c.finish();
        n.anchored = vec![
            Anchored {
                cwd: false,
                line: 4,
                rel: "../..".into(),
                pattern: ".env*".into(),
                conditional: false,
            },
            Anchored {
                cwd: true,
                line: 5,
                rel: "".into(),
                pattern: EVERYTHING_BELOW.into(),
                conditional: true,
            },
        ];
        let fact = n.encode();
        assert_eq!(
            fact,
            "names:0,0,0,0,0\n3\tfixtures/a.json\n@L\t4\t../..\t.env*\n@C?\t5\t\t**"
        );
        let d = decode(&fact).unwrap();
        assert_eq!(
            d.names().collect::<Vec<_>>(),
            [(3, "fixtures/a.json", None)]
        );
        assert_eq!(d.anchored().collect::<Vec<_>>(), n.anchored);
        // Entries alone are not nothing.
        let only = Names {
            anchored: n.anchored.clone(),
            ..Names::default()
        };
        assert!(!only.is_empty());
        assert!(!decode(&only.encode()).unwrap().is_empty());
    }
}
