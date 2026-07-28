//! Fast-path parser for pnpm lockfiles built directly on YAML parse events.
//!
//! `serde_yaml_ng` (backed by `unsafe-libyaml`) spends the majority of pnpm
//! lockfile parsing inside the YAML scanner, and its serde layer adds
//! buffering overhead for the `#[serde(flatten)]`/`#[serde(untagged)]`
//! shapes in [`PnpmLockfile`]. This module builds the same structs from
//! `saphyr-parser` events, which scan roughly twice as fast.
//!
//! Correctness strategy: this parser is deliberately conservative. Anything
//! outside the regular, machine-generated subset of YAML that pnpm emits —
//! anchors, aliases, tags, multiple documents, block scalars, duplicate
//! keys, exotic numeric forms — aborts the fast path by returning `None`,
//! and the caller falls back to the serde path. Both paths must produce
//! equal (`==`) lockfiles for any input the fast path accepts; tests
//! enforce this differentially.

use std::collections::BTreeMap;

use saphyr_parser::{Event, Parser, ScalarStyle, StrInput};
use serde_yaml_ng::{Mapping, Number, Value};

#[path = "data_scanner.rs"]
mod scanner;

use super::{
    DependenciesMeta, Dependency, DependencyInfo, LockfileSettings, Map, PackageResolution,
    PackageSnapshot, PackageSnapshotV7, Packages, PatchFile, PnpmLockfile, ProjectSnapshot,
    Snapshots,
};
use crate::pnpm::LockfileVersion;

/// Marker for "this input is outside the supported fast-path subset".
/// Callers fall back to the serde parser; this is not an error condition.
struct Unsupported;

impl Unsupported {
    /// Construct while optionally tracing the bail location. Tracing is
    /// compiled in but effectively free unless debug logging is enabled;
    /// knowing *why* a lockfile leaves the fast path matters in the field.
    #[track_caller]
    fn here() -> Self {
        if tracing::enabled!(tracing::Level::DEBUG) {
            let loc = std::panic::Location::caller();
            tracing::debug!(
                "pnpm fast parse unsupported at {}:{}",
                loc.file(),
                loc.line()
            );
        }
        Unsupported
    }
}

type FResult<T> = Result<T, Unsupported>;

/// Attempt to parse a pnpm lockfile from YAML using the fast paths.
/// Tier 1 is a structural line scanner specialized to the pnpm subset;
/// tier 2 replays the input through the general saphyr event parser.
/// Returns `None` when the input uses YAML constructs outside the
/// supported subset (or is malformed); the caller must then use the serde
/// path, which either parses it or reports a proper error.
pub(super) fn parse(bytes: &[u8]) -> Option<PnpmLockfile> {
    let text = std::str::from_utf8(bytes).ok()?;
    parse_with_scanner(text).or_else(|| parse_with_saphyr(text))
}

fn parse_with_scanner(text: &str) -> Option<PnpmLockfile> {
    if text.len() >= PARALLEL_PARSE_MIN_BYTES
        && let Some(lockfile) = parse_fragments_parallel(text)
    {
        return Some(lockfile);
    }
    // Unsplittable, small, or rejected-by-fragment input: sequential scan
    // over the whole document, exactly as before. This keeps the parallel
    // path a pure optimization: anything it bails on gets the same
    // treatment the input would have gotten without it.
    let mut events = Events {
        source: Source::Lines(scanner::LineScanner::new(text)),
        peeked: None,
    };
    parse_lockfile(&mut events).ok()
}

/// Minimum input size for the parallel path; below this the sequential
/// scan wins because rayon dispatch and per-fragment stream framing cost
/// more than they save.
const PARALLEL_PARSE_MIN_BYTES: usize = 64 * 1024;

/// Byte slices of the document's top-level blocks: each fragment starts at
/// a column-zero key line and runs to the next one, so it is itself a
/// valid single-key document for the line scanner.
///
/// The split is purely positional and deliberately conservative: it only
/// recognizes boundaries whose first byte could begin a plain pnpm map key
/// (alphanumeric or `_`). Anything else at column zero — document markers,
/// flow collections, quoted keys, tabs — returns `None` and the caller
/// falls back to the sequential scan. Comment and blank lines are not
/// boundaries; they attach to the preceding fragment, where the scanner
/// skips them.
///
/// The first fragment always starts at byte zero so nothing is dropped:
/// leading comments are skipped by the scanner, and leading *content*
/// (e.g. an indented first line) makes the fragment scan fail exactly like
/// the sequential scan would.
fn split_top_level(text: &str) -> Option<Vec<&str>> {
    let bytes = text.as_bytes();
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            b' ' | b'\n' | b'\r' | b'#' => {}
            c if c.is_ascii_alphanumeric() || c == b'_' => starts.push(pos),
            _ => return None,
        }
        pos = match memchr::memchr(b'\n', &bytes[pos..]) {
            Some(i) => pos + i + 1,
            None => bytes.len(),
        };
    }
    if starts.len() < 2 {
        return None;
    }
    let mut fragments = Vec::with_capacity(starts.len());
    let mut begin = 0;
    for &start in &starts[1..] {
        fragments.push(&text[begin..start]);
        begin = start;
    }
    fragments.push(&text[begin..]);
    Some(fragments)
}

/// Scan the document's top-level blocks in parallel and merge the results.
/// The `packages`, `snapshots`, and `importers` blocks are comparable in
/// size and dominate parse time, so this approaches a 3x speedup on large
/// lockfiles. Returns `None` when the input isn't worth splitting or any
/// fragment uses YAML the scanner doesn't support.
fn parse_fragments_parallel(text: &str) -> Option<PnpmLockfile> {
    use rayon::prelude::*;

    let fragments = split_top_level(text)?;
    let parsed = fragments
        .into_par_iter()
        .map(parse_fragment)
        .collect::<FResult<Vec<_>>>()
        .ok()?;

    let mut fields = TopLevelFields::default();
    for fragment_fields in parsed {
        fields.merge(fragment_fields).ok()?;
    }
    fields.into_lockfile().ok()
}

/// Minimum fragment size before it's worth re-splitting into chunks of
/// child entries.
const CHUNKED_FRAGMENT_MIN_BYTES: usize = 256 * 1024;

/// Parse one top-level block. Fragments large enough to dominate the
/// parallel schedule (`importers`/`packages`/`snapshots` on big repos) are
/// re-split at their child keys and parsed as concurrent chunks; anything
/// else — or any chunk the scanner rejects — is parsed sequentially so the
/// chunked path stays a pure optimization.
fn parse_fragment(fragment: &str) -> FResult<TopLevelFields> {
    if fragment.len() >= CHUNKED_FRAGMENT_MIN_BYTES
        && let Some(fields) = parse_fragment_chunked(fragment, rayon::current_num_threads().max(2))
    {
        return Ok(fields);
    }
    let mut events = Events {
        source: Source::Lines(scanner::LineScanner::new(fragment)),
        peeked: None,
    };
    let mut fields = TopLevelFields::default();
    parse_root_mapping(&mut events, &mut fields)?;
    Ok(fields)
}

/// Byte offsets of child-entry starts: lines indented by exactly two
/// spaces whose first content byte could begin a pnpm map key (package
/// keys like `'@scope/name@1.0.0':`, importer paths like `packages/foo` or
/// `.`). `-` is excluded so block-sequence items never look like keys, and
/// any line indented by exactly one space aborts (the two-space level
/// would not be a direct child). Continuation lines of multi-line scalars
/// are indented deeper than two spaces, so a boundary can never land
/// inside an entry the scanner accepts.
fn child_entry_starts(fragment: &str) -> Option<Vec<usize>> {
    let bytes = fragment.as_bytes();
    let mut starts = Vec::new();
    // Skip the `key:` header line.
    let mut pos = memchr::memchr(b'\n', bytes)? + 1;
    while pos < bytes.len() {
        let line_start = pos;
        pos = match memchr::memchr(b'\n', &bytes[pos..]) {
            Some(i) => pos + i + 1,
            None => bytes.len(),
        };
        match bytes[line_start] {
            b'\n' | b'\r' | b'#' => continue,
            b' ' => {}
            // Content at column zero after the header: not a splittable
            // block (split_top_level only produces this via CR quirks).
            _ => return None,
        }
        if line_start + 1 >= bytes.len() {
            continue;
        }
        match bytes[line_start + 1] {
            b' ' => {}
            // Indent of exactly one: two-space lines would not be direct
            // children, so give up on chunking this fragment.
            _ => return None,
        }
        let Some(&third) = bytes.get(line_start + 2) else {
            continue;
        };
        match third {
            // Deeper indentation or blank remainder: interior line.
            b' ' | b'\n' | b'\r' => continue,
            c if c.is_ascii_alphanumeric()
                || matches!(c, b'_' | b'\'' | b'"' | b'@' | b'.' | b'/') =>
            {
                starts.push(line_start)
            }
            // `-` (sequence item), `#` (comment), tags, anchors, flow:
            // don't risk chunking.
            _ => return None,
        }
    }
    Some(starts)
}

/// Parse a big top-level block by splitting its children into roughly
/// core-count chunks, parsing `header + chunk` mini-documents in parallel,
/// and unioning the resulting single-field accumulators. The union rejects
/// duplicate child keys exactly like the sequential parsers'
/// insert-checks, and duplicate scalar fields via `set_once`, so accepted
/// inputs mean the same thing they would sequentially.
fn parse_fragment_chunked(fragment: &str, chunk_count: usize) -> Option<TopLevelFields> {
    use rayon::prelude::*;

    let header_end = memchr::memchr(b'\n', fragment.as_bytes())? + 1;
    let header = &fragment[..header_end];
    // Only the sections whose sequential parsers reject duplicate child
    // keys with an insert-check, which the chunk union mirrors exactly.
    if !matches!(header.trim_end(), "importers:" | "packages:" | "snapshots:") {
        return None;
    }
    let starts = child_entry_starts(fragment)?;
    if starts.len() < chunk_count * 2 {
        return None;
    }
    let target_bytes = fragment.len() / chunk_count;

    // Group child starts into chunks of at least `target_bytes`.
    let mut chunk_bounds = vec![starts[0]];
    let mut chunk_begin = starts[0];
    for &start in &starts[1..] {
        if start - chunk_begin >= target_bytes {
            chunk_bounds.push(start);
            chunk_begin = start;
        }
    }
    chunk_bounds.push(fragment.len());

    let parsed = chunk_bounds
        .windows(2)
        .map(|w| (w[0], w[1]))
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(begin, end)| {
            let doc = format!("{header}{}", &fragment[begin..end]);
            let mut events = Events {
                source: Source::Lines(scanner::LineScanner::new(&doc)),
                peeked: None,
            };
            let mut fields = TopLevelFields::default();
            parse_root_mapping(&mut events, &mut fields).map(|()| fields)
        })
        .collect::<FResult<Vec<_>>>()
        .ok()?;

    let mut merged = TopLevelFields::default();
    for chunk_fields in parsed {
        merged.union(chunk_fields).ok()?;
    }
    Some(merged)
}

fn parse_with_saphyr(text: &str) -> Option<PnpmLockfile> {
    let mut events = Events {
        source: Source::Saphyr(Box::new(Parser::new_from_str(text))),
        peeked: None,
    };
    parse_lockfile(&mut events).ok()
}

enum Source<'a> {
    Saphyr(Box<Parser<'a, StrInput<'a>>>),
    Lines(scanner::LineScanner<'a>),
}

struct Events<'a> {
    source: Source<'a>,
    peeked: Option<Event<'a>>,
}

impl<'a> Events<'a> {
    fn next(&mut self) -> FResult<Event<'a>> {
        if let Some(ev) = self.peeked.take() {
            return Ok(ev);
        }
        match &mut self.source {
            Source::Saphyr(parser) => match parser.next() {
                Some(Ok((ev, _span))) => Ok(ev),
                Some(Err(_)) | None => Err(Unsupported::here()),
            },
            Source::Lines(scanner) => scanner.next_event(),
        }
    }

    fn peek(&mut self) -> FResult<&Event<'a>> {
        if self.peeked.is_none() {
            self.peeked = Some(self.next()?);
        }
        self.peeked.as_ref().ok_or(Unsupported)
    }

    /// Consume the next event and require it to be a scalar without anchor
    /// or tag, returning its decoded text and style. Block scalars (`|`,
    /// `>`) are fine here: the parser hands us the spec-decoded value, and
    /// serde treats any non-plain scalar as a string (pnpm emits folded
    /// scalars for long `deprecated` messages).
    fn scalar(&mut self) -> FResult<(String, ScalarStyle)> {
        match self.next()? {
            Event::Scalar(value, style, anchor_id, tag) => {
                if anchor_id != 0 || tag.is_some() {
                    return Err(Unsupported::here());
                }
                Ok((value.into_owned(), style))
            }
            _ => Err(Unsupported::here()),
        }
    }

    /// Consume a scalar in a string-typed position (map key, `String`
    /// field, `Map<String, String>` value). Mirrors serde: a plain scalar
    /// that reads as YAML null fails string deserialization, so it aborts
    /// the fast path rather than silently becoming a string.
    fn string(&mut self) -> FResult<String> {
        let (value, style) = self.scalar()?;
        if style == ScalarStyle::Plain && is_yaml_null(&value) {
            return Err(Unsupported::here());
        }
        Ok(value)
    }

    fn mapping_start(&mut self) -> FResult<()> {
        match self.next()? {
            Event::MappingStart(anchor_id, tag) if anchor_id == 0 && tag.is_none() => Ok(()),
            _ => Err(Unsupported::here()),
        }
    }

    /// True when the next event ends the current mapping (consumes it).
    fn at_mapping_end(&mut self) -> FResult<bool> {
        if matches!(self.peek()?, Event::MappingEnd) {
            self.next()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Skip a complete node (scalar, sequence, or mapping) without
    /// interpreting it. Used for unknown fields that serde would ignore.
    fn skip_node(&mut self) -> FResult<()> {
        let mut depth = 0usize;
        loop {
            match self.next()? {
                Event::Scalar(_, _, anchor_id, _) => {
                    if anchor_id != 0 {
                        return Err(Unsupported::here());
                    }
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Event::SequenceStart(anchor_id, _) | Event::MappingStart(anchor_id, _) => {
                    if anchor_id != 0 {
                        return Err(Unsupported::here());
                    }
                    depth += 1;
                }
                Event::SequenceEnd | Event::MappingEnd => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Event::Alias(_) => return Err(Unsupported::here()),
                _ => return Err(Unsupported::here()),
            }
        }
    }
}

fn is_yaml_null(scalar: &str) -> bool {
    matches!(scalar, "" | "null" | "Null" | "NULL" | "~")
}

fn parse_bool_scalar(value: &str, style: ScalarStyle) -> FResult<bool> {
    if style != ScalarStyle::Plain {
        return Err(Unsupported::here());
    }
    match value {
        "true" | "True" | "TRUE" => Ok(true),
        "false" | "False" | "FALSE" => Ok(false),
        _ => Err(Unsupported::here()),
    }
}

/// Convert a plain scalar to a `serde_yaml_ng::Value`, mirroring
/// `serde_yaml_ng`'s untagged scalar typing for the common subset. Exotic
/// forms it types differently (hex/octal/binary ints, `+` prefixes,
/// `.inf`/`.nan`, 128-bit integers) abort the fast path.
fn plain_scalar_to_value(value: String) -> FResult<Value> {
    if is_yaml_null(&value) {
        return Ok(Value::Null);
    }
    match value.as_str() {
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        _ => {}
    }
    let bytes = value.as_bytes();
    let first = bytes[0];

    // Exotic prefixes serde parses as numbers: bail instead of matching its
    // radix handling.
    if value.starts_with('+')
        || value.starts_with("0x")
        || value.starts_with("0o")
        || value.starts_with("0b")
        || value.starts_with("-0x")
        || value.starts_with("-0o")
        || value.starts_with("-0b")
    {
        return Err(Unsupported::here());
    }
    if matches!(
        value.as_str(),
        ".inf" | ".Inf" | ".INF" | "-.inf" | "-.Inf" | "-.INF" | ".nan" | ".NaN" | ".NAN"
    ) {
        return Err(Unsupported::here());
    }

    let digits = if first == b'-' { &bytes[1..] } else { bytes };
    let all_digits = !digits.is_empty() && digits.iter().all(|b| b.is_ascii_digit());
    // YAML 1.2: leading zeros make it a string, not a number.
    let leading_zero_string = digits.len() > 1 && digits[0] == b'0';

    if all_digits && !leading_zero_string {
        if first == b'-' {
            if let Ok(int) = value.parse::<i64>() {
                return Ok(Value::Number(Number::from(int)));
            }
        } else if let Ok(int) = value.parse::<u64>() {
            return Ok(Value::Number(Number::from(int)));
        }
        // Out of 64-bit range: serde falls through to i128/u128 handling.
        return Err(Unsupported::here());
    }

    if !(all_digits && leading_zero_string)
        && let Ok(float) = value.parse::<f64>()
        && float.is_finite()
    {
        return Ok(Value::Number(Number::from(float)));
    }

    Ok(Value::String(value))
}

/// Build a `serde_yaml_ng::Value` from events for unknown subtrees that
/// round-trip through `other` fields.
fn parse_value(events: &mut Events) -> FResult<Value> {
    match events.next()? {
        Event::Scalar(value, style, anchor_id, tag) => {
            if anchor_id != 0 || tag.is_some() {
                return Err(Unsupported::here());
            }
            match style {
                ScalarStyle::Plain => plain_scalar_to_value(value.into_owned()),
                // Quoted and block scalars are always strings under serde's
                // untagged typing.
                _ => Ok(Value::String(value.into_owned())),
            }
        }
        Event::SequenceStart(anchor_id, tag) => {
            if anchor_id != 0 || tag.is_some() {
                return Err(Unsupported::here());
            }
            let mut seq = Vec::new();
            while !matches!(events.peek()?, Event::SequenceEnd) {
                seq.push(parse_value(events)?);
            }
            events.next()?;
            Ok(Value::Sequence(seq))
        }
        Event::MappingStart(anchor_id, tag) => {
            if anchor_id != 0 || tag.is_some() {
                return Err(Unsupported::here());
            }
            let mut mapping = Mapping::new();
            loop {
                if matches!(events.peek()?, Event::MappingEnd) {
                    events.next()?;
                    break;
                }
                let key = parse_value(events)?;
                let value = parse_value(events)?;
                // serde_yaml_ng errors on duplicate mapping keys when
                // deserializing a Value; mirror by falling back.
                if mapping.insert(key, value).is_some() {
                    return Err(Unsupported::here());
                }
            }
            Ok(Value::Mapping(mapping))
        }
        _ => Err(Unsupported::here()),
    }
}

/// Parse `Map<String, String>` (BTreeMap semantics: duplicate keys last-win
/// under serde, but pnpm never emits duplicates; abort to stay exact).
fn parse_string_map(events: &mut Events) -> FResult<BTreeMap<String, String>> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        let value = events.string()?;
        if map.insert(key, value).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_string_seq(events: &mut Events) -> FResult<Vec<String>> {
    match events.next()? {
        Event::SequenceStart(anchor_id, tag) if anchor_id == 0 && tag.is_none() => {}
        _ => return Err(Unsupported::here()),
    }
    let mut seq = Vec::new();
    while !matches!(events.peek()?, Event::SequenceEnd) {
        seq.push(events.string()?);
    }
    events.next()?;
    Ok(seq)
}

/// A dependency section of an importer, before deciding between the
/// pre-v6 (`name -> version` strings) and v6 (`name -> {specifier,
/// version}`) representations.
enum DepSection {
    Strings(BTreeMap<String, String>),
    Structured(BTreeMap<String, Dependency>),
    /// `{}` — compatible with either representation.
    Empty,
}

fn parse_dep_section(events: &mut Events) -> FResult<DepSection> {
    events.mapping_start()?;
    if events.at_mapping_end()? {
        return Ok(DepSection::Empty);
    }
    let first_key = events.string()?;
    match events.peek()? {
        Event::Scalar(..) => {
            let mut map = BTreeMap::new();
            map.insert(first_key, events.string()?);
            while !events.at_mapping_end()? {
                let key = events.string()?;
                if map.insert(key, events.string()?).is_some() {
                    return Err(Unsupported::here());
                }
            }
            Ok(DepSection::Strings(map))
        }
        Event::MappingStart(..) => {
            let mut map = BTreeMap::new();
            map.insert(first_key, parse_dependency(events)?);
            while !events.at_mapping_end()? {
                let key = events.string()?;
                if map.insert(key, parse_dependency(events)?).is_some() {
                    return Err(Unsupported::here());
                }
            }
            Ok(DepSection::Structured(map))
        }
        _ => Err(Unsupported::here()),
    }
}

fn parse_dependency(events: &mut Events) -> FResult<Dependency> {
    events.mapping_start()?;
    let mut specifier = None;
    let mut version = None;
    while !events.at_mapping_end()? {
        let key = events.string()?;
        match key.as_str() {
            "specifier" => set_once(&mut specifier, events.string()?)?,
            "version" => set_once(&mut version, events.string()?)?,
            _ => events.skip_node()?,
        }
    }
    // Both fields are required by the serde struct; missing means the serde
    // path errors, so fall back and let it produce that error.
    Ok(Dependency {
        specifier: specifier.ok_or_else(Unsupported::here)?,
        version: version.ok_or_else(Unsupported::here)?,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> FResult<()> {
    // serde rejects duplicate struct fields.
    if slot.replace(value).is_some() {
        return Err(Unsupported::here());
    }
    Ok(())
}

fn parse_project_snapshot(events: &mut Events) -> FResult<ProjectSnapshot> {
    events.mapping_start()?;
    let mut specifiers = None;
    let mut dependencies = None;
    let mut optional_dependencies = None;
    let mut dev_dependencies = None;
    let mut dependencies_meta = None;
    let mut publish_directory = None;

    while !events.at_mapping_end()? {
        let key = events.string()?;
        match key.as_str() {
            "specifiers" => set_once(&mut specifiers, parse_string_map(events)?)?,
            "dependencies" => set_once(&mut dependencies, parse_dep_section(events)?)?,
            "optionalDependencies" => {
                set_once(&mut optional_dependencies, parse_dep_section(events)?)?
            }
            "devDependencies" => set_once(&mut dev_dependencies, parse_dep_section(events)?)?,
            "dependenciesMeta" => {
                set_once(&mut dependencies_meta, parse_dependencies_meta(events)?)?
            }
            "publishDirectory" => set_once(&mut publish_directory, events.string()?)?,
            // ProjectSnapshot's flatten chain silently ignores unknown keys.
            _ => events.skip_node()?,
        }
    }

    // Mirror serde's untagged resolution order for DependencyInfo: PreV6 is
    // declared first, so it wins whenever every present section deserializes
    // as plain strings (including when all are absent or empty). A single
    // structured section forces V6; a conflict (structured + strings) fails
    // both variants under serde, so fall back.
    let any_structured = [&dependencies, &optional_dependencies, &dev_dependencies]
        .into_iter()
        .flatten()
        .any(|s| matches!(s, DepSection::Structured(_)));
    let any_strings = [&dependencies, &optional_dependencies, &dev_dependencies]
        .into_iter()
        .flatten()
        .any(|s| matches!(s, DepSection::Strings(_)));
    if any_structured && any_strings {
        return Err(Unsupported::here());
    }
    if any_structured && specifiers.is_some() {
        return Err(Unsupported::here());
    }

    let dependencies_info = if any_structured {
        DependencyInfo::V6 {
            dependencies: dependencies.map(into_structured).transpose()?,
            optional_dependencies: optional_dependencies.map(into_structured).transpose()?,
            dev_dependencies: dev_dependencies.map(into_structured).transpose()?,
        }
    } else {
        DependencyInfo::PreV6 {
            specifiers,
            dependencies: dependencies.map(into_strings).transpose()?,
            optional_dependencies: optional_dependencies.map(into_strings).transpose()?,
            dev_dependencies: dev_dependencies.map(into_strings).transpose()?,
        }
    };

    Ok(ProjectSnapshot {
        dependencies: dependencies_info,
        dependencies_meta,
        publish_directory,
    })
}

fn into_strings(section: DepSection) -> FResult<BTreeMap<String, String>> {
    match section {
        DepSection::Strings(map) => Ok(map),
        DepSection::Empty => Ok(BTreeMap::new()),
        DepSection::Structured(_) => Err(Unsupported::here()),
    }
}

fn into_structured(section: DepSection) -> FResult<BTreeMap<String, Dependency>> {
    match section {
        DepSection::Structured(map) => Ok(map),
        DepSection::Empty => Ok(BTreeMap::new()),
        DepSection::Strings(_) => Err(Unsupported::here()),
    }
}

fn parse_dependencies_meta(events: &mut Events) -> FResult<Map<String, DependenciesMeta>> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        events.mapping_start()?;
        let mut injected = None;
        let mut node = None;
        let mut patch = None;
        while !events.at_mapping_end()? {
            let field = events.string()?;
            match field.as_str() {
                "injected" => {
                    let (value, style) = events.scalar()?;
                    set_once(&mut injected, parse_bool_scalar(&value, style)?)?;
                }
                "node" => set_once(&mut node, events.string()?)?,
                "patch" => set_once(&mut patch, events.string()?)?,
                _ => events.skip_node()?,
            }
        }
        if map
            .insert(
                key,
                DependenciesMeta {
                    injected,
                    node,
                    patch,
                },
            )
            .is_some()
        {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_resolution(events: &mut Events) -> FResult<PackageResolution> {
    events.mapping_start()?;
    let mut type_field = None;
    let mut integrity = None;
    let mut tarball = None;
    let mut directory = None;
    let mut repo = None;
    let mut commit = None;
    // Unknown resolution fields (e.g. the `variants` list of a
    // `type: variations` runtime resolution) must round-trip through `other`
    // rather than be dropped, mirroring the serde flatten catch-all on
    // `PackageResolution`.
    let mut other = Map::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        match key.as_str() {
            "type" => set_once(&mut type_field, events.string()?)?,
            "integrity" => set_once(&mut integrity, events.string()?)?,
            "tarball" => set_once(&mut tarball, events.string()?)?,
            "directory" => set_once(&mut directory, events.string()?)?,
            "repo" => set_once(&mut repo, events.string()?)?,
            "commit" => set_once(&mut commit, events.string()?)?,
            _ => {
                let value = parse_value(events)?;
                if other.insert(key, value).is_some() {
                    return Err(Unsupported::here());
                }
            }
        }
    }
    Ok(PackageResolution {
        type_field,
        integrity,
        tarball,
        directory,
        repo,
        commit,
        other,
    })
}

/// Fields shared by `packages` entries (which flatten a
/// [`PackageSnapshotV7`]) and `snapshots` entries.
struct SnapshotV7Fields {
    optional: Option<bool>,
    dependencies: Option<Map<String, String>>,
    optional_dependencies: Option<Map<String, String>>,
    transitive_peer_dependencies: Option<Vec<String>>,
}

impl SnapshotV7Fields {
    fn new() -> Self {
        Self {
            optional: None,
            dependencies: None,
            optional_dependencies: None,
            transitive_peer_dependencies: None,
        }
    }

    /// Try to consume a known V7 snapshot field. Returns false when the key
    /// isn't part of the snapshot shape.
    fn consume(&mut self, key: &str, events: &mut Events) -> FResult<bool> {
        match key {
            "optional" => {
                let (value, style) = events.scalar()?;
                set_once(&mut self.optional, parse_bool_scalar(&value, style)?)?;
            }
            "dependencies" => set_once(&mut self.dependencies, parse_string_map(events)?)?,
            "optionalDependencies" => {
                set_once(&mut self.optional_dependencies, parse_string_map(events)?)?
            }
            "transitivePeerDependencies" => set_once(
                &mut self.transitive_peer_dependencies,
                parse_string_seq(events)?,
            )?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn build(self) -> PackageSnapshotV7 {
        PackageSnapshotV7 {
            optional: self.optional.unwrap_or_default(),
            dependencies: self.dependencies,
            optional_dependencies: self.optional_dependencies,
            transitive_peer_dependencies: self.transitive_peer_dependencies,
        }
    }
}

fn parse_package_snapshot(events: &mut Events) -> FResult<PackageSnapshot> {
    events.mapping_start()?;
    let mut resolution = None;
    let mut id = None;
    let mut name = None;
    let mut version = None;
    let mut patched = None;
    let mut v7 = SnapshotV7Fields::new();
    let mut other = Map::new();

    while !events.at_mapping_end()? {
        let key = events.string()?;
        match key.as_str() {
            "resolution" => set_once(&mut resolution, parse_resolution(events)?)?,
            "id" => set_once(&mut id, events.string()?)?,
            "name" => set_once(&mut name, events.string()?)?,
            "version" => set_once(&mut version, events.string()?)?,
            "patched" => {
                let (value, style) = events.scalar()?;
                set_once(&mut patched, parse_bool_scalar(&value, style)?)?;
            }
            _ => {
                if !v7.consume(&key, events)? {
                    let value = parse_value(events)?;
                    if other.insert(key, value).is_some() {
                        return Err(Unsupported::here());
                    }
                }
            }
        }
    }

    Ok(PackageSnapshot {
        resolution: resolution.ok_or_else(Unsupported::here)?,
        id,
        name,
        version,
        snapshot: v7.build(),
        patched,
        other,
    })
}

fn parse_snapshot_v7(events: &mut Events) -> FResult<PackageSnapshotV7> {
    events.mapping_start()?;
    let mut v7 = SnapshotV7Fields::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        if !v7.consume(&key, events)? {
            events.skip_node()?;
        }
    }
    Ok(v7.build())
}

fn parse_packages(events: &mut Events) -> FResult<Packages> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        if map.insert(key, parse_package_snapshot(events)?).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_snapshots(events: &mut Events) -> FResult<Snapshots> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        if map.insert(key, parse_snapshot_v7(events)?).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_importers(events: &mut Events) -> FResult<BTreeMap<String, ProjectSnapshot>> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        if map.insert(key, parse_project_snapshot(events)?).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_patched_dependencies(events: &mut Events) -> FResult<Map<String, PatchFile>> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        let patch = match events.peek()? {
            Event::Scalar(..) => PatchFile::Hash(events.string()?),
            Event::MappingStart(..) => {
                events.mapping_start()?;
                let mut path = None;
                let mut hash = None;
                while !events.at_mapping_end()? {
                    let field = events.string()?;
                    match field.as_str() {
                        "path" => set_once(&mut path, events.string()?)?,
                        "hash" => set_once(&mut hash, events.string()?)?,
                        _ => events.skip_node()?,
                    }
                }
                PatchFile::PathAndHash {
                    path: path.ok_or_else(Unsupported::here)?,
                    hash: hash.ok_or_else(Unsupported::here)?,
                }
            }
            _ => return Err(Unsupported::here()),
        };
        if map.insert(key, patch).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_catalogs(events: &mut Events) -> FResult<Map<String, Map<String, Dependency>>> {
    events.mapping_start()?;
    let mut map = BTreeMap::new();
    while !events.at_mapping_end()? {
        let key = events.string()?;
        events.mapping_start()?;
        let mut catalog = BTreeMap::new();
        while !events.at_mapping_end()? {
            let name = events.string()?;
            if catalog.insert(name, parse_dependency(events)?).is_some() {
                return Err(Unsupported::here());
            }
        }
        if map.insert(key, catalog).is_some() {
            return Err(Unsupported::here());
        }
    }
    Ok(map)
}

fn parse_settings(events: &mut Events) -> FResult<LockfileSettings> {
    events.mapping_start()?;
    let mut auto_install_peers = None;
    let mut exclude_links_from_lockfile = None;
    let mut inject_workspace_packages = None;
    let mut dedupe_peers = None;
    let mut peers_suffix_max_length = None;
    while !events.at_mapping_end()? {
        let key = events.string()?;
        match key.as_str() {
            "autoInstallPeers" => {
                let (value, style) = events.scalar()?;
                set_once(&mut auto_install_peers, parse_bool_scalar(&value, style)?)?;
            }
            "excludeLinksFromLockfile" => {
                let (value, style) = events.scalar()?;
                set_once(
                    &mut exclude_links_from_lockfile,
                    parse_bool_scalar(&value, style)?,
                )?;
            }
            "injectWorkspacePackages" => {
                let (value, style) = events.scalar()?;
                set_once(
                    &mut inject_workspace_packages,
                    parse_bool_scalar(&value, style)?,
                )?;
            }
            "dedupePeers" => {
                let (value, style) = events.scalar()?;
                set_once(&mut dedupe_peers, parse_bool_scalar(&value, style)?)?;
            }
            "peersSuffixMaxLength" => {
                let (value, style) = events.scalar()?;
                if style != ScalarStyle::Plain {
                    return Err(Unsupported::here());
                }
                set_once(
                    &mut peers_suffix_max_length,
                    value.parse::<u32>().map_err(|_| Unsupported::here())?,
                )?;
            }
            _ => events.skip_node()?,
        }
    }
    Ok(LockfileSettings {
        auto_install_peers,
        exclude_links_from_lockfile,
        inject_workspace_packages,
        dedupe_peers,
        peers_suffix_max_length,
    })
}

fn parse_lockfile_version(events: &mut Events) -> FResult<LockfileVersion> {
    let (value, style) = events.scalar()?;
    match style {
        // Mirror serde's untagged StringOrNum: a plain scalar deserializes
        // as f32 when possible (`5.4` → Float format via f32::to_string).
        ScalarStyle::Plain => match value.parse::<f32>() {
            Ok(num) => Ok(LockfileVersion::from(num)),
            Err(_) => Ok(LockfileVersion::from(value)),
        },
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => Ok(LockfileVersion::from(value)),
        _ => Err(Unsupported::here()),
    }
}

/// Top-level lockfile fields accumulated while parsing the root mapping.
/// The parallel path (see [`parse_fragments_parallel`]) fills one
/// accumulator per top-level block and merges them; `set_once` in both
/// `consume` and `merge` keeps duplicate-key behavior identical to a
/// sequential pass over the whole document.
#[derive(Default)]
struct TopLevelFields {
    lockfile_version: Option<LockfileVersion>,
    settings: Option<LockfileSettings>,
    catalogs: Option<Map<String, Map<String, Dependency>>>,
    pnpmfile_checksum: Option<String>,
    never_built_dependencies: Option<Vec<String>>,
    only_built_dependencies: Option<Vec<String>>,
    ignored_optional_dependencies: Option<Vec<String>>,
    overrides: Option<Map<String, String>>,
    package_extensions_checksum: Option<String>,
    patched_dependencies: Option<Map<String, PatchFile>>,
    importers: Option<BTreeMap<String, ProjectSnapshot>>,
    packages: Option<Packages>,
    snapshots: Option<Snapshots>,
    time: Option<Map<String, String>>,
}

impl TopLevelFields {
    /// Parse the value for one root-mapping key.
    fn consume(&mut self, key: &str, events: &mut Events) -> FResult<()> {
        match key {
            "lockfileVersion" => {
                set_once(&mut self.lockfile_version, parse_lockfile_version(events)?)?
            }
            "settings" => set_once(&mut self.settings, parse_settings(events)?)?,
            "catalogs" => set_once(&mut self.catalogs, parse_catalogs(events)?)?,
            "pnpmfileChecksum" => set_once(&mut self.pnpmfile_checksum, events.string()?)?,
            "neverBuiltDependencies" => set_once(
                &mut self.never_built_dependencies,
                parse_string_seq(events)?,
            )?,
            "onlyBuiltDependencies" => {
                set_once(&mut self.only_built_dependencies, parse_string_seq(events)?)?
            }
            "ignoredOptionalDependencies" => set_once(
                &mut self.ignored_optional_dependencies,
                parse_string_seq(events)?,
            )?,
            "overrides" => set_once(&mut self.overrides, parse_string_map(events)?)?,
            "packageExtensionsChecksum" => {
                set_once(&mut self.package_extensions_checksum, events.string()?)?
            }
            "patchedDependencies" => set_once(
                &mut self.patched_dependencies,
                parse_patched_dependencies(events)?,
            )?,
            "importers" => set_once(&mut self.importers, parse_importers(events)?)?,
            "packages" => set_once(&mut self.packages, parse_packages(events)?)?,
            "snapshots" => set_once(&mut self.snapshots, parse_snapshots(events)?)?,
            "time" => set_once(&mut self.time, parse_string_map(events)?)?,
            // Unknown root keys are ignored by the serde struct.
            _ => events.skip_node()?,
        }
        Ok(())
    }

    /// Fold another accumulator into this one, rejecting duplicate fields
    /// exactly like repeated keys in a single pass would be.
    fn merge(&mut self, other: TopLevelFields) -> FResult<()> {
        fn fold<T>(slot: &mut Option<T>, value: Option<T>) -> FResult<()> {
            match value {
                Some(value) => set_once(slot, value),
                None => Ok(()),
            }
        }
        fold(&mut self.lockfile_version, other.lockfile_version)?;
        fold(&mut self.settings, other.settings)?;
        fold(&mut self.catalogs, other.catalogs)?;
        fold(&mut self.pnpmfile_checksum, other.pnpmfile_checksum)?;
        fold(
            &mut self.never_built_dependencies,
            other.never_built_dependencies,
        )?;
        fold(
            &mut self.only_built_dependencies,
            other.only_built_dependencies,
        )?;
        fold(
            &mut self.ignored_optional_dependencies,
            other.ignored_optional_dependencies,
        )?;
        fold(&mut self.overrides, other.overrides)?;
        fold(
            &mut self.package_extensions_checksum,
            other.package_extensions_checksum,
        )?;
        fold(&mut self.patched_dependencies, other.patched_dependencies)?;
        fold(&mut self.importers, other.importers)?;
        fold(&mut self.packages, other.packages)?;
        fold(&mut self.snapshots, other.snapshots)?;
        fold(&mut self.time, other.time)?;
        Ok(())
    }

    /// Fold chunks of a single re-split section back together. Map
    /// sections union with a duplicate-key check, mirroring the sequential
    /// parsers' insert-checks; every other field uses `set_once`, exactly
    /// like [`TopLevelFields::merge`].
    fn union(&mut self, other: TopLevelFields) -> FResult<()> {
        fn union_map<V>(
            slot: &mut Option<BTreeMap<String, V>>,
            value: Option<BTreeMap<String, V>>,
        ) -> FResult<()> {
            let Some(value) = value else {
                return Ok(());
            };
            match slot {
                None => *slot = Some(value),
                Some(existing) => {
                    for (key, entry) in value {
                        if existing.insert(key, entry).is_some() {
                            return Err(Unsupported::here());
                        }
                    }
                }
            }
            Ok(())
        }
        let mut other = other;
        union_map(&mut self.importers, other.importers.take())?;
        union_map(&mut self.packages, other.packages.take())?;
        union_map(&mut self.snapshots, other.snapshots.take())?;
        // Chunked fragments only carry the three sections above; merge the
        // rest anyway so this stays correct if the whitelist ever grows.
        self.merge(other)
    }

    fn into_lockfile(self) -> FResult<PnpmLockfile> {
        Ok(PnpmLockfile {
            lockfile_version: self.lockfile_version.ok_or_else(Unsupported::here)?,
            cached_version: Default::default(),
            leading_documents: Vec::new(),
            settings: self.settings,
            catalogs: self.catalogs,
            pnpmfile_checksum: self.pnpmfile_checksum,
            never_built_dependencies: self.never_built_dependencies,
            only_built_dependencies: self.only_built_dependencies,
            ignored_optional_dependencies: self.ignored_optional_dependencies,
            overrides: self.overrides,
            package_extensions_checksum: self.package_extensions_checksum,
            patched_dependencies: self.patched_dependencies,
            // `importers` has no `#[serde(default)]`; a missing field is a
            // serde error, so let the fallback path report it.
            importers: self.importers.ok_or_else(Unsupported::here)?,
            packages: self.packages,
            snapshots: self.snapshots,
            dependency_index: rustc_hash::FxHashMap::default(),
            time: self.time,
        })
    }
}

/// Parse one document containing a root block mapping into `fields`.
/// Consumes the entire event stream: used both for whole documents and for
/// single top-level blocks re-framed as standalone documents.
fn parse_root_mapping(events: &mut Events, fields: &mut TopLevelFields) -> FResult<()> {
    // StreamStart, then exactly one document.
    match events.next()? {
        Event::StreamStart => {}
        _ => return Err(Unsupported::here()),
    }
    match events.next()? {
        Event::DocumentStart(_) => {}
        _ => return Err(Unsupported::here()),
    }

    events.mapping_start()?;

    while !events.at_mapping_end()? {
        let key = events.string()?;
        fields.consume(&key, events)?;
    }

    // Exactly one document: anything after DocumentEnd other than stream end
    // means multi-document input (leading documents), handled by the serde
    // path.
    match events.next()? {
        Event::DocumentEnd => {}
        _ => return Err(Unsupported::here()),
    }
    match events.next()? {
        Event::StreamEnd => {}
        _ => return Err(Unsupported::here()),
    }
    Ok(())
}

fn parse_lockfile(events: &mut Events) -> FResult<PnpmLockfile> {
    let mut fields = TopLevelFields::default();
    parse_root_mapping(events, &mut fields)?;
    fields.into_lockfile()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Parse via the fast path (applying the same post-parse steps as
    /// `from_bytes`) and via the serde oracle; both must succeed and agree.
    /// When the structural scanner accepts the input, its result must agree
    /// with the oracle too.
    fn assert_fast_matches_serde(yaml: &str) {
        let mut fast = parse(yaml.as_bytes()).expect("fast path must accept this input");
        fast.cached_version = fast.compute_version();
        fast.build_dependency_index();
        let serde = PnpmLockfile::from_bytes_via_serde(yaml.as_bytes())
            .expect("serde path must accept this input");
        assert_eq!(fast, serde);

        // Round-trip fidelity must match too: prune re-encodes lockfiles.
        assert_eq!(
            String::from_utf8(crate::Lockfile::encode(&fast).expect("fast encodes")).ok(),
            String::from_utf8(crate::Lockfile::encode(&serde).expect("serde encodes")).ok(),
        );

        if let Some(mut scanned) = parse_with_scanner(yaml) {
            scanned.cached_version = scanned.compute_version();
            scanned.build_dependency_index();
            assert_eq!(scanned, serde, "scanner tier must agree with serde");
        }

        // The parallel splitter is size-gated in production; run it here on
        // every corpus input so its accept/agree behavior is checked
        // differentially just like the tiers above.
        if let Some(mut split) = parse_fragments_parallel(yaml) {
            split.cached_version = split.compute_version();
            split.build_dependency_index();
            assert_eq!(split, serde, "parallel split must agree with serde");
        }
    }

    /// Like [`assert_fast_matches_serde`], but additionally requires the
    /// structural scanner tier to accept the input. Guards against silent
    /// regressions where mainline pnpm shapes start falling through to the
    /// slower tiers.
    fn assert_scanner_matches_serde(yaml: &str) {
        assert!(
            parse_with_scanner(yaml).is_some(),
            "structural scanner must accept this input"
        );
        assert_fast_matches_serde(yaml);
    }

    fn assert_falls_back(yaml: &str) {
        assert!(
            parse(yaml.as_bytes()).is_none(),
            "input should be outside the fast-path subset"
        );
        // The parallel splitter must not accept anything the sequential
        // tiers reject: at production sizes it runs first, so accepting
        // here would widen `parse`'s accepted language.
        assert!(
            parse_fragments_parallel(yaml).is_none(),
            "parallel split must reject inputs outside the fast-path subset"
        );
    }

    #[test]
    fn test_v9_lockfile_differential() {
        assert_scanner_matches_serde(
            r#"lockfileVersion: '9.0'

settings:
  autoInstallPeers: true
  excludeLinksFromLockfile: false

catalogs:
  default:
    react:
      specifier: ^19.0.0
      version: 19.0.0

overrides:
  semver: 7.5.3

patchedDependencies:
  is-even@1.0.0:
    hash: abcdef123456
    path: patches/is-even@1.0.0.patch
  is-odd: legacyhashonly

importers:

  .:
    dependencies:
      express:
        specifier: 4.18.2
        version: 4.18.2
    devDependencies:
      turbo:
        specifier: canary
        version: 2.0.0

  apps/empty: {}

  apps/meta:
    dependencies:
      ui:
        specifier: workspace:*
        version: link:../../packages/ui
    dependenciesMeta:
      ui:
        injected: true
    publishDirectory: dist

packages:

  express@4.18.2:
    resolution: {integrity: sha512-aaa}
    engines: {node: '>= 0.10.0'}
    hasBin: true
    deprecated: use something else
    cpu: [x64, arm64]
    peerDependencies:
      react: '>=16'
    peerDependenciesMeta:
      react:
        optional: true

  turbo@2.0.0:
    resolution: {integrity: sha512-bbb}
    version: 2.0.0-canary.1
    name: turbo
    optional: true

snapshots:

  express@4.18.2:
    dependencies:
      body-parser: 1.20.1
    transitivePeerDependencies:
      - supports-color

  turbo@2.0.0:
    optional: true
"#,
        );
    }

    #[test]
    fn test_v5_lockfile_differential() {
        assert_scanner_matches_serde(
            r#"lockfileVersion: 5.4

importers:

  .:
    specifiers:
      lodash: ^4.17.21
    dependencies:
      lodash: 4.17.21

packages:

  /lodash/4.17.21:
    resolution: {integrity: sha512-ccc}
    dev: false
"#,
        );
    }

    #[test]
    fn test_v6_lockfile_differential() {
        assert_scanner_matches_serde(
            r#"lockfileVersion: '6.0'

importers:

  .:
    dependencies:
      chalk:
        specifier: ^5.0.0
        version: 5.3.0

packages:

  /chalk@5.3.0:
    resolution: {integrity: sha512-ddd}
    engines: {node: ^12.17.0 || ^14.13 || >=16.0.0}
    dev: false
"#,
        );
    }

    #[test]
    fn test_time_and_checksums_differential() {
        assert_scanner_matches_serde(
            r#"lockfileVersion: '9.0'
pnpmfileChecksum: abc123
packageExtensionsChecksum: def456
neverBuiltDependencies:
  - fsevents
onlyBuiltDependencies:
  - esbuild
importers:
  .: {}
time:
  /lodash/4.17.21: '2021-02-20T15:42:16.891Z'
"#,
        );
    }

    #[test]
    fn test_unknown_fields_are_ignored_like_serde() {
        assert_fast_matches_serde(
            r#"lockfileVersion: '9.0'
someFutureRootField:
  nested: [1, two, false]
importers:
  .:
    someFutureImporterField: hello
packages:
  a@1.0.0:
    resolution: {integrity: sha512-a, futureResolutionField: x}
    futureCustomField:
      deeply:
        nested: true
"#,
        );
    }

    #[test]
    fn test_scalar_typing_in_other_fields() {
        // `other` passthrough values must type plain scalars exactly like
        // serde_yaml_ng: ints, floats, bools, nulls, leading-zero strings.
        assert_fast_matches_serde(
            r#"lockfileVersion: '9.0'
importers:
  .: {}
packages:
  a@1.0.0:
    resolution: {integrity: sha512-a}
    someInt: 42
    someNegative: -7
    someFloat: 1.5
    someBool: true
    someNull: ~
    leadingZeros: 0123
    quotedNumber: '42'
    someList: [1, 2.5, x]
"#,
        );
    }

    #[test]
    fn test_block_scalars_differential() {
        // pnpm emits folded scalars for long deprecated messages. Both
        // parsers must decode block scalars (including chomping variants)
        // to identical strings.
        assert_fast_matches_serde(
            r#"lockfileVersion: '9.0'
importers:
  .: {}
packages:
  a@1.0.0:
    resolution: {integrity: sha512-a}
    deprecated: >-
      This package has been deprecated in favor of something-else.
      Please migrate at your earliest convenience.
  b@1.0.0:
    resolution: {integrity: sha512-b}
    deprecated: |
      literal block
      keeps newlines
  c@1.0.0:
    resolution: {integrity: sha512-c}
    deprecated: |+
      keep trailing

  d@1.0.0:
    resolution: {integrity: sha512-d}
    deprecated: >
      folded with trailing newline
"#,
        );
    }

    #[test]
    fn test_multi_document_falls_back() {
        assert_falls_back("---\nleading: doc\n---\nlockfileVersion: '9.0'\nimporters:\n  .: {}\n");
    }

    #[test]
    fn test_anchors_fall_back() {
        assert_falls_back("lockfileVersion: '9.0'\nimporters:\n  .: {}\nx: &a hello\ny: *a\n");
    }

    #[test]
    fn test_duplicate_keys_fall_back() {
        assert_falls_back("lockfileVersion: '9.0'\nimporters:\n  .: {}\n  .: {}\n");
    }

    #[test]
    fn test_hex_int_falls_back() {
        assert_falls_back(
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    resolution: \
             {integrity: sha512-a}\n    weird: 0x2A\n",
        );
    }

    #[test]
    fn test_null_in_string_position_falls_back() {
        // serde errors on `~` where a String is expected; the fast path must
        // not silently produce a different result.
        assert_falls_back("lockfileVersion: '9.0'\nimporters:\n  .: {}\noverrides:\n  foo: ~\n");
    }

    #[test]
    fn test_missing_importers_falls_back() {
        assert_falls_back("lockfileVersion: '9.0'\n");
    }

    #[test]
    fn test_scanner_rejects_what_saphyr_rejects() {
        // `a: b: c` is a YAML error; accepting it would fabricate a
        // lockfile where the serde path reports an error.
        assert!(
            parse_with_scanner("lockfileVersion: '9.0'\nimporters:\n  .: {}\nx: a: b\n").is_none()
        );
        // Plain value ending in a colon is likewise invalid.
        assert!(
            parse_with_scanner("lockfileVersion: '9.0'\nimporters:\n  .: {}\nx: a:\n").is_none()
        );
    }

    #[test]
    fn test_scanner_empty_flow_and_null_values() {
        assert_scanner_matches_serde("lockfileVersion: '9.0'\nimporters:\n  .: {}\n");
    }

    #[test]
    fn test_scanner_folded_scalars_differential() {
        assert_scanner_matches_serde(
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    resolution: {integrity: sha512-a}\n    deprecated: >-\n      This package is deprecated. Use\n      something else instead.\n\n      Second paragraph here.\n  b@1.0.0:\n    resolution: {integrity: sha512-b}\n    deprecated: >\n      trailing newline kept\n",
        );
    }

    #[test]
    fn test_scanner_literal_scalars_differential() {
        // Literal blocks (`|`, `|-`) keep line breaks and more-indented
        // lines verbatim; next.js's lockfile uses `|-` for deprecation
        // messages.
        assert_scanner_matches_serde(
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    resolution: \
             {integrity: sha512-a}\n    deprecated: |-\n      line one\n      line two\n\n      \
             after a blank\n        more indented kept verbatim\n  b@1.0.0:\n    resolution: \
             {integrity: sha512-b}\n    deprecated: |\n      clipped keeps one newline\n",
        );
        // A blank separator line after a block scalar (pnpm emits one
        // between package entries) is discarded by both strip and clip
        // chomping.
        assert_scanner_matches_serde(
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    resolution: \
             {integrity: sha512-a}\n    deprecated: |-\n      last line\n\n  b@1.0.0:\n    \
             resolution: {integrity: sha512-b}\n    deprecated: >-\n      folded last\n\n    \
             engines: {node: '>=10'}\n",
        );
        // `|+` keep-chomping stays outside the scanner subset.
        assert!(
            parse_with_scanner(
                "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    \
                 resolution: {integrity: sha512-a}\n    deprecated: |+\n      kept\n\n"
            )
            .is_none()
        );
    }

    #[test]
    fn test_scanner_sequence_forms_differential() {
        // Block sequence at key indent and deeper, plus flow sequences.
        assert_scanner_matches_serde(
            "lockfileVersion: '9.0'\nneverBuiltDependencies:\n- \
             fsevents\nonlyBuiltDependencies:\n  - esbuild\nimporters:\n  .: {}\npackages:\n  \
             a@1.0.0:\n    resolution: {integrity: sha512-a}\n    os: [darwin, linux]\n    cpu: \
             [x64]\n",
        );
    }

    #[test]
    fn test_scanner_quoting_differential() {
        assert_scanner_matches_serde(
            "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      '@scope/pkg':\n        specifier: '>=1.0.0'\n        version: 1.0.0\npackages:\n  '@scope/pkg@1.0.0':\n    resolution: {integrity: sha512-a, tarball: 'https://example.com/x, y.tgz'}\n    weird: 'it''s quoted'\n    dquote: \"plain dq\"\n",
        );
    }

    #[test]
    fn test_scanner_comment_and_crlf_handling() {
        // Full-line comments are meaning-preserving to skip.
        assert_scanner_matches_serde("lockfileVersion: '9.0'\n# a comment\nimporters:\n  .: {}\n");
        // Inline comments and CRLF are outside the scanner subset but fine
        // for the later tiers.
        let inline_comment = "lockfileVersion: '9.0'\nimporters:\n  .: {}\nx: y # trailing\n";
        assert!(parse_with_scanner(inline_comment).is_none());
        assert_fast_matches_serde(inline_comment);
        let crlf = "lockfileVersion: '9.0'\r\nimporters:\r\n  .: {}\r\n";
        assert!(parse_with_scanner(crlf).is_none());
        assert_fast_matches_serde(crlf);
    }

    #[test]
    fn test_repo_own_lockfile_differential() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let lockfile_path = std::path::Path::new(manifest_dir)
            .join("../..")
            .join("pnpm-lock.yaml");
        let bytes = std::fs::read(&lockfile_path).expect("repo lockfile readable");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        assert_scanner_matches_serde(text);
    }

    /// A lockfile large enough to cross the production size gates, so
    /// `parse` runs the parallel splitter and the chunked section parsers
    /// for real.
    fn synthetic_large_lockfile() -> String {
        let mut yaml =
            String::from("lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: true\n");
        yaml.push_str("\nimporters:\n\n  .:\n    dependencies:\n      dep-0:\n        specifier: 1.0.0\n        version: 1.0.0\n");
        for i in 0..6000 {
            yaml.push_str(&format!(
                "\n  packages/pkg-{i}:\n    dependencies:\n      dep-{i}:\n        specifier: \
                 ^{i}.0.0\n        version: {i}.4.2\n"
            ));
        }
        yaml.push_str("\npackages:\n");
        for i in 0..6000 {
            yaml.push_str(&format!(
                "\n  dep-{i}@{i}.4.2:\n    resolution: {{integrity: sha512-abc{i}}}\n"
            ));
        }
        yaml.push_str("\nsnapshots:\n");
        for i in 0..6000 {
            yaml.push_str(&format!(
                "\n  dep-{i}@{i}.4.2:\n    dependencies:\n      other-{i}: {i}.0.0\n"
            ));
        }
        yaml
    }

    #[test]
    fn test_parallel_split_large_lockfile_differential() {
        let yaml = synthetic_large_lockfile();
        assert!(yaml.len() >= PARALLEL_PARSE_MIN_BYTES);
        assert!(
            parse_fragments_parallel(&yaml).is_some(),
            "parallel splitter must accept a mainline large lockfile"
        );
        assert_scanner_matches_serde(&yaml);

        // The big sections must be over the chunking threshold so the
        // assertion above exercised the chunked path, not just the
        // top-level split.
        let fragments = split_top_level(&yaml).expect("splittable");
        let packages = fragments
            .iter()
            .find(|f| f.starts_with("packages:"))
            .expect("packages fragment");
        assert!(packages.len() >= CHUNKED_FRAGMENT_MIN_BYTES);
    }

    #[test]
    fn test_split_top_level_fragments() {
        let yaml = "a: 1\n# comment\nb:\n  x: y\n\nc: 2\n";
        let fragments = split_top_level(yaml).expect("splittable");
        // Comments and blank lines attach to the preceding fragment.
        assert_eq!(
            fragments,
            vec!["a: 1\n# comment\n", "b:\n  x: y\n\n", "c: 2\n"]
        );
        // Reassembly is lossless by construction.
        assert_eq!(fragments.concat(), yaml);

        // Column-zero characters that can't start a plain key refuse to
        // split: document markers, flow collections, directives.
        assert_eq!(split_top_level("---\na: 1\nb: 2\n"), None);
        assert_eq!(split_top_level("{a: 1}\nb: 2\n"), None);
        assert_eq!(split_top_level("%YAML 1.2\na: 1\nb: 2\n"), None);
        // A single block can't be split.
        assert_eq!(split_top_level("a:\n  b: c\n"), None);
    }

    #[test]
    fn test_parallel_split_duplicate_top_level_sections_rejected() {
        // Duplicate root keys land in different fragments; the merge must
        // reject them just like `set_once` does sequentially.
        let yaml = "lockfileVersion: '9.0'\nimporters:\n  .: {}\npackages:\n  a@1.0.0:\n    \
                    resolution: {integrity: sha512-a}\npackages:\n  b@1.0.0:\n    resolution: \
                    {integrity: sha512-b}\n";
        assert!(parse_fragments_parallel(yaml).is_none());
    }

    #[test]
    fn test_chunked_fragment_matches_sequential() {
        let mut fragment = String::from("packages:\n");
        for i in 0..12 {
            fragment.push_str(&format!(
                "\n  dep-{i}@1.0.{i}:\n    resolution: {{integrity: sha512-abc{i}}}\n"
            ));
        }
        let chunked = parse_fragment_chunked(&fragment, 3).expect("chunkable");
        let Ok(sequential) = parse_fragment(&fragment) else {
            panic!("fragment must parse sequentially");
        };
        assert_eq!(chunked.packages, sequential.packages);
        assert_eq!(
            chunked.packages.as_ref().map(|map| map.len()),
            Some(12),
            "all children accounted for across chunk boundaries"
        );
    }

    #[test]
    fn test_chunked_fragment_duplicate_child_keys_rejected() {
        let mut fragment = String::from("packages:\n");
        for i in 0..12 {
            fragment.push_str(&format!(
                "\n  dep-{i}@1.0.{i}:\n    resolution: {{integrity: sha512-abc{i}}}\n"
            ));
        }
        // Duplicate an early key at the end so the copies land in
        // different chunks and only the union check can catch them.
        fragment.push_str("\n  dep-0@1.0.0:\n    resolution: {integrity: sha512-dup}\n");
        assert!(parse_fragment_chunked(&fragment, 3).is_none());
        // The sequential parser rejects them too, so behavior is identical
        // with or without chunking.
        assert!(parse_fragment(&fragment).is_err());
    }

    #[test]
    fn test_chunking_restricted_to_known_sections() {
        let mut fragment = String::from("time:\n");
        for i in 0..12 {
            fragment.push_str(&format!("  dep-{i}: '2024-01-0{}'\n", i % 9 + 1));
        }
        assert!(parse_fragment_chunked(&fragment, 3).is_none());
    }
}
