use std::{borrow::Cow, fmt, sync::Arc, time::SystemTime};

use serde::{Serialize, Serializer, ser::SerializeMap};

/// Strip control characters and ANSI escape sequences from a string.
///
/// ANSI escape sequences are consumed as complete units so that
/// orphaned parameter bytes never appear in output. Handled:
///
/// - **CSI** (`ESC [` and C1 `U+009B`): parameter + final byte consumed
/// - **OSC** (`ESC ]`): text consumed until BEL or ST
/// - **Fe/Fp/Fs** (`ESC` + `0x30..=0x7E`): two-byte sequences
///
/// When `preserve_newlines` is true, `\n` is kept. When false, all
/// control characters (ASCII C0, DEL, and Unicode C1) are removed.
///
/// Returns `Cow::Borrowed` when no changes are needed.
fn strip_control_chars(input: &str, preserve_newlines: bool) -> Cow<'_, str> {
    let needs_work = input
        .chars()
        .any(|c| c.is_control() && (!preserve_newlines || c != '\n'));

    if !needs_work {
        return Cow::Borrowed(input);
    }

    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ESC — consume the full ANSI escape sequence.
            match chars.peek().copied() {
                Some('[') => {
                    // CSI: ESC [ <params> <final byte 0x40..=0x7E per ECMA-48>
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: ESC ] <text> <BEL | ST>
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek().copied() == Some('\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Fe (0x40-0x5F), Fp (0x30-0x3F), Fs (0x60-0x7E):
                // two-character escape sequences. CSI '[' and OSC ']'
                // are already matched above.
                Some(c2) if ('0'..='~').contains(&c2) => {
                    chars.next();
                }
                _ => {} // Standalone ESC
            }
            continue;
        }

        // C1 CSI (U+009B): single-byte equivalent of ESC [.
        // Strip the introducer and consume parameter/final bytes
        // the same way as the ESC [ branch above.
        if c == '\u{009b}' {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
            continue;
        }

        // Strip ASCII C0, DEL, and Unicode C1 control characters.
        if c.is_control() {
            if preserve_newlines && c == '\n' {
                result.push(c);
            }
            continue;
        }

        result.push(c);
    }

    Cow::Owned(result)
}

/// Sanitize a user-provided message string.
///
/// Strips control characters and ANSI escape sequences while
/// preserving newlines (multi-line messages are common).
fn sanitize_message(input: String) -> String {
    match strip_control_chars(&input, true) {
        Cow::Borrowed(_) => input,
        Cow::Owned(sanitized) => sanitized,
    }
}

/// Severity level for user-facing log events.
///
/// Ordered by severity: `Info < Warn < Error`. This matches the standard
/// convention where `Error` is the most severe.
///
/// There is intentionally no `Debug` level. Debug-level output is
/// developer-facing and should use the `tracing` crate instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    /// Informational messages.
    Info,
    /// Warnings that don't prevent progress.
    Warn,
    /// Errors that affect correctness or indicate failure.
    Error,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Level::Error => write!(f, "ERROR"),
            Level::Warn => write!(f, "WARN"),
            Level::Info => write!(f, "INFO"),
        }
    }
}

/// A Turborepo infrastructure subsystem that can emit log events.
///
/// Each variant identifies a logical area of the codebase. Adding a
/// variant here is the only way to register a new subsystem — this
/// keeps the set discoverable and typo-proof at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case")]
pub enum Subsystem {
    /// Package boundary checks (`turbo boundaries`).
    Boundaries,
    /// Cache reads, writes, and configuration.
    Cache,
    /// Log file replay and related output.
    Logs,
    /// The `turbo run` orchestrator.
    Run,
    /// Source control (git) operations.
    Scm,
    /// Global shim version-mismatch warnings.
    Shim,
    /// Run summary generation and output.
    Summary,
    /// Task-access permission checks.
    TaskAccess,
    /// Tracing/logging infrastructure messages.
    Tracing,
}

impl fmt::Display for Subsystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subsystem::Boundaries => write!(f, "boundaries"),
            Subsystem::Cache => write!(f, "cache"),
            Subsystem::Logs => write!(f, "logs"),
            Subsystem::Run => write!(f, "run"),
            Subsystem::Scm => write!(f, "scm"),
            Subsystem::Shim => write!(f, "shim"),
            Subsystem::Summary => write!(f, "summary"),
            Subsystem::TaskAccess => write!(f, "task-access"),
            Subsystem::Tracing => write!(f, "tracing"),
        }
    }
}

/// Origin of a log event.
///
/// # Relationship to `turborepo-task-id`
///
/// [`Source::Task`] deliberately uses `Arc<str>` rather than depending on
/// `turborepo_task_id::TaskId` to keep this crate's dependency footprint
/// minimal (it sits at the bottom of the dependency graph). Callers with
/// a `TaskId` should pass `task_id.to_string()` to [`Source::task()`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// Turborepo infrastructure (cache, scm, run, etc.).
    Turbo(Subsystem),
    /// A specific task, identified by its display string (e.g., `"web#build"`).
    /// Uses `Arc<str>` so cloning a `LogHandle` for a task is cheap.
    Task(Arc<str>),
}

// Manual Serialize impl to avoid requiring serde's "rc" feature for Arc<str>.
impl Serialize for Source {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Source::Turbo(subsystem) => {
                serializer.serialize_newtype_variant("Source", 0, "Turbo", subsystem)
            }
            Source::Task(id) => serializer.serialize_newtype_variant("Source", 1, "Task", &**id),
        }
    }
}

impl Source {
    /// Create a source identifying a Turborepo subsystem.
    pub fn turbo(subsystem: Subsystem) -> Self {
        Source::Turbo(subsystem)
    }

    /// Create a source identifying a specific task.
    ///
    /// Accepts any string-like type. Task IDs typically follow the
    /// `package#task` format (e.g., `"web#build"`).
    ///
    /// All control characters — including newlines — are stripped to
    /// prevent terminal injection and ensure task IDs remain single-line.
    /// Full ANSI escape sequences are consumed as units.
    pub fn task(id: impl AsRef<str>) -> Self {
        let cleaned = strip_control_chars(id.as_ref(), false);
        Source::Task(Arc::from(cleaned.as_ref()))
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Turbo(subsystem) => write!(f, "turbo:{subsystem}"),
            Source::Task(id) => write!(f, "task:{id}"),
        }
    }
}

/// A string guaranteed to be free of control characters and ANSI escape
/// sequences.
///
/// Construct via the [`From`] impls (which sanitize) or
/// [`SanitizedString::from_trusted`] for strings that are already
/// known-clean (e.g., numeric conversions).
///
/// Terminal-rendering sinks can trust `SanitizedString` content without
/// re-sanitizing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SanitizedString(String);

impl SanitizedString {
    /// Create from a string that is already known to be clean.
    ///
    /// No sanitization is performed. Use this only for strings that
    /// are guaranteed to contain no control characters or ANSI escape
    /// sequences (e.g., numeric conversions, compile-time constants).
    pub fn from_trusted(s: String) -> Self {
        Self(s)
    }

    /// The inner string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume and return the inner string.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for SanitizedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for SanitizedString {
    fn eq(&self, other: &str) -> bool {
        self.0 == *other
    }
}

/// Sanitize: control characters and ANSI escape sequences are stripped.
/// Newlines are also removed (field values should be single-line).
impl From<&str> for SanitizedString {
    fn from(s: &str) -> Self {
        match strip_control_chars(s, false) {
            Cow::Borrowed(clean) => Self(clean.to_owned()),
            Cow::Owned(sanitized) => Self(sanitized),
        }
    }
}

/// Sanitize: control characters and ANSI escape sequences are stripped.
/// Newlines are also removed (field values should be single-line).
impl From<String> for SanitizedString {
    fn from(s: String) -> Self {
        match strip_control_chars(&s, false) {
            Cow::Borrowed(_) => Self(s),
            Cow::Owned(sanitized) => Self(sanitized),
        }
    }
}

/// A non-recursive structured field value.
///
/// Used inside [`Value::List`] to enforce flat lists at the type level.
/// All scalar variants of [`Value`] have a corresponding `Scalar` variant.
///
/// `Scalar` implements `PartialEq` but not `Eq` because `Float(f64)`
/// does not satisfy IEEE 754 reflexivity (`NaN != NaN`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(untagged)]
pub enum Scalar {
    /// A sanitized UTF-8 string.
    String(SanitizedString),
    /// A signed 64-bit integer.
    Int(i64),
    /// A boolean.
    Bool(bool),
    /// A 64-bit float.
    Float(f64),
    /// A redacted value (serializes as JSON `null`).
    #[serde(serialize_with = "serialize_redacted")]
    Redacted,
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::String(s) => write!(f, "{s}"),
            Scalar::Int(n) => write!(f, "{n}"),
            Scalar::Bool(b) => write!(f, "{b}"),
            Scalar::Float(n) => write!(f, "{n}"),
            Scalar::Redacted => write!(f, "[REDACTED]"),
        }
    }
}

impl From<&str> for Scalar {
    fn from(s: &str) -> Self {
        Scalar::String(SanitizedString::from(s))
    }
}

impl From<String> for Scalar {
    fn from(s: String) -> Self {
        Scalar::String(SanitizedString::from(s))
    }
}

impl From<i64> for Scalar {
    fn from(n: i64) -> Self {
        Scalar::Int(n)
    }
}

/// Values above `i64::MAX` are stored as `Scalar::String` to avoid
/// silent truncation.
impl From<u64> for Scalar {
    fn from(n: u64) -> Self {
        match i64::try_from(n) {
            Ok(signed) => Scalar::Int(signed),
            Err(_) => Scalar::String(SanitizedString::from_trusted(n.to_string())),
        }
    }
}

/// Values above `i64::MAX` are stored as `Scalar::String` to avoid
/// silent truncation.
impl From<usize> for Scalar {
    fn from(n: usize) -> Self {
        match i64::try_from(n) {
            Ok(signed) => Scalar::Int(signed),
            Err(_) => Scalar::String(SanitizedString::from_trusted(n.to_string())),
        }
    }
}

impl From<i32> for Scalar {
    fn from(n: i32) -> Self {
        Scalar::Int(n as i64)
    }
}

impl From<u32> for Scalar {
    fn from(n: u32) -> Self {
        Scalar::Int(n as i64)
    }
}

impl From<bool> for Scalar {
    fn from(b: bool) -> Self {
        Scalar::Bool(b)
    }
}

impl From<f64> for Scalar {
    fn from(n: f64) -> Self {
        Scalar::Float(n)
    }
}

/// A structured field value for log event metadata.
///
/// This is a restricted set of types suitable for structured logging fields.
///
/// # Sanitization
///
/// String values use [`SanitizedString`], which strips control characters
/// and ANSI escape sequences. The [`From`] impls construct sanitized
/// strings automatically. The sanitization invariant is enforced at the
/// type level — terminal-rendering sinks can trust string content
/// without re-sanitizing.
///
/// # `PartialEq` without `Eq`
///
/// `Value` implements `PartialEq` but not `Eq` because `Float(f64)`
/// does not satisfy IEEE 754 reflexivity (`NaN != NaN`). This means
/// `Value` cannot be used as a `HashMap` key or in contexts requiring
/// `Eq`. If this is needed, encode floats as `Value::String`.
///
/// # Flat lists
///
/// `Value::List` holds [`Vec<Scalar>`] rather than `Vec<Value>`,
/// enforcing flat lists at the type level. This prevents stack overflow
/// during serialization from recursive nesting.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(untagged)]
pub enum Value {
    /// A sanitized UTF-8 string.
    String(SanitizedString),
    /// A signed 64-bit integer.
    Int(i64),
    /// A boolean.
    Bool(bool),
    /// A 64-bit float.
    Float(f64),
    /// A flat list of scalar values. Holds [`Scalar`] rather than
    /// `Value` to prevent recursive nesting.
    List(Vec<Scalar>),
    /// A redacted value. Serializes as JSON `null` (not the string
    /// `"[REDACTED]"`) so that log consumers can distinguish redacted
    /// fields from fields that literally contain that text.
    ///
    /// The `Display` impl still renders as `[REDACTED]` for
    /// human-readable output.
    #[serde(serialize_with = "serialize_redacted")]
    Redacted,
}

fn serialize_redacted<S: Serializer>(s: S) -> Result<S::Ok, S::Error> {
    s.serialize_none()
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::String(s) => write!(f, "{s}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Float(n) => write!(f, "{n}"),
            Value::List(items) => {
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                Ok(())
            }
            Value::Redacted => write!(f, "[REDACTED]"),
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(SanitizedString::from(s))
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(SanitizedString::from(s))
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}

/// Values above `i64::MAX` are stored as `Value::String` to avoid
/// silent truncation.
impl From<u64> for Value {
    fn from(n: u64) -> Self {
        match i64::try_from(n) {
            Ok(signed) => Value::Int(signed),
            Err(_) => Value::String(SanitizedString::from_trusted(n.to_string())),
        }
    }
}

/// Values above `i64::MAX` are stored as `Value::String` to avoid
/// silent truncation.
impl From<usize> for Value {
    fn from(n: usize) -> Self {
        match i64::try_from(n) {
            Ok(signed) => Value::Int(signed),
            Err(_) => Value::String(SanitizedString::from_trusted(n.to_string())),
        }
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Int(n as i64)
    }
}

impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Int(n as i64)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Float(n)
    }
}

impl From<Scalar> for Value {
    fn from(s: Scalar) -> Self {
        match s {
            Scalar::String(s) => Value::String(s),
            Scalar::Int(n) => Value::Int(n),
            Scalar::Bool(b) => Value::Bool(b),
            Scalar::Float(n) => Value::Float(n),
            Scalar::Redacted => Value::Redacted,
        }
    }
}

impl<T: Into<Scalar>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Self {
        Value::List(items.into_iter().map(Into::into).collect())
    }
}

/// Serialize fields as a JSON object rather than an array of tuples.
///
/// Duplicate keys are preserved in insertion order. Most JSON parsers
/// use the last occurrence when duplicates are present, but behavior
/// is parser-dependent per RFC 8259 §4.
fn serialize_fields<S: Serializer>(
    fields: &[(&'static str, Value)],
    s: S,
) -> Result<S::Ok, S::Error> {
    let mut map = s.serialize_map(Some(fields.len()))?;
    for (k, v) in fields {
        map.serialize_entry(k, v)?;
    }
    map.end()
}

/// Serialize `SystemTime` as milliseconds since the Unix epoch.
///
/// Produces a single integer (e.g., `1710345600000`) rather than
/// serde's default `{"secs_since_epoch": N, "nanos_since_epoch": N}`
/// struct. This format is understood by virtually all log tooling.
fn serialize_timestamp<S: Serializer>(ts: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let millis = ts
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    s.serialize_u64(u64::try_from(millis).unwrap_or(u64::MAX))
}

/// A structured log event for user-facing output.
///
/// Created via [`LogHandle`](crate::LogHandle) methods or the free functions
/// [`warn`](crate::warn), [`info`](crate::info), [`error`](crate::error).
/// Sink implementations receive these and decide how to render them.
///
/// # Field access
///
/// Sink implementations access event data via the public accessor
/// methods ([`level()`](Self::level), [`message()`](Self::message),
/// etc.). Fields are `pub(crate)` to enforce construction through
/// [`LogEvent::new`] or the builder API, which sanitize the message.
#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub(crate) level: Level,
    pub(crate) source: Source,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(serialize_with = "serialize_fields")]
    pub(crate) fields: Vec<(&'static str, Value)>,
    #[serde(serialize_with = "serialize_timestamp")]
    pub(crate) timestamp: SystemTime,
}

impl LogEvent {
    /// Create a new event with the current timestamp and no fields.
    ///
    /// The message is sanitized: control characters (except newline) and
    /// ANSI escape sequences are stripped.
    pub fn new(level: Level, source: Source, message: impl Into<String>) -> Self {
        Self {
            level,
            source,
            message: sanitize_message(message.into()),
            fields: Vec::new(),
            timestamp: SystemTime::now(),
        }
    }

    /// Create a new event with an explicit timestamp.
    ///
    /// Useful in tests where deterministic timestamps are needed.
    pub fn with_timestamp(
        level: Level,
        source: Source,
        message: impl Into<String>,
        timestamp: SystemTime,
    ) -> Self {
        Self {
            level,
            source,
            message: sanitize_message(message.into()),
            fields: Vec::new(),
            timestamp,
        }
    }

    /// Severity of this event.
    pub fn level(&self) -> Level {
        self.level
    }

    /// Origin — which subsystem or task produced this.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Human-readable message text. Sanitized on construction: control
    /// characters (except newline) and ANSI escape sequences are stripped.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Structured key-value metadata. Keys are `&'static str`, values are
    /// [`Value`]. Insertion order is preserved. Serializes as a JSON object.
    pub fn fields(&self) -> &[(&'static str, Value)] {
        &self.fields
    }

    /// When this event was created. Serializes as milliseconds since the
    /// Unix epoch.
    pub fn timestamp(&self) -> SystemTime {
        self.timestamp
    }

    /// Append a structured field.
    pub(crate) fn push_field(&mut self, key: &'static str, value: Value) {
        self.fields.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering_matches_severity() {
        assert!(Level::Error > Level::Warn);
        assert!(Level::Warn > Level::Info);
        assert!(Level::Error > Level::Info);
    }

    #[test]
    fn level_display() {
        assert_eq!(Level::Error.to_string(), "ERROR");
        assert_eq!(Level::Warn.to_string(), "WARN");
        assert_eq!(Level::Info.to_string(), "INFO");
    }

    #[test]
    fn level_serializes_uppercase() {
        assert_eq!(serde_json::to_string(&Level::Info).unwrap(), "\"INFO\"");
        assert_eq!(serde_json::to_string(&Level::Warn).unwrap(), "\"WARN\"");
        assert_eq!(serde_json::to_string(&Level::Error).unwrap(), "\"ERROR\"");
    }

    #[test]
    fn source_display() {
        assert_eq!(Source::turbo(Subsystem::Cache).to_string(), "turbo:cache");
        assert_eq!(Source::task("web#build").to_string(), "task:web#build");
    }

    #[test]
    fn source_task_strips_full_ansi_escape_sequences() {
        let source = Source::task("evil\x1b[31mtask\r\n");
        match &source {
            Source::Task(id) => assert_eq!(id.as_ref(), "eviltask"),
            _ => panic!("expected Task variant"),
        }
    }

    #[test]
    fn source_task_strips_osc_sequences() {
        let source = Source::task("before\x1b]0;title\x07after");
        match &source {
            Source::Task(id) => assert_eq!(id.as_ref(), "beforeafter"),
            _ => panic!("expected Task variant"),
        }
    }

    #[test]
    fn source_task_strips_newlines() {
        let source = Source::task("line1\nline2");
        match &source {
            Source::Task(id) => assert_eq!(id.as_ref(), "line1line2"),
            _ => panic!("expected Task variant"),
        }
    }

    #[test]
    fn source_task_preserves_clean_strings() {
        let source = Source::task("@scope/pkg#build");
        assert_eq!(source.to_string(), "task:@scope/pkg#build");
    }

    #[test]
    fn sanitized_string_strips_control_chars() {
        let s = SanitizedString::from("\x1b[31mred\x1b[0m");
        assert_eq!(s.as_str(), "red");

        let s = SanitizedString::from("with\nnewline");
        assert_eq!(s.as_str(), "withnewline");

        let s = SanitizedString::from("clean string");
        assert_eq!(s.as_str(), "clean string");
    }

    #[test]
    fn sanitized_string_from_trusted_preserves_content() {
        let raw = SanitizedString::from_trusted("\x1b[31mred\x1b[0m".to_string());
        assert!(raw.as_str().contains("\x1b[31m"));
    }

    #[test]
    fn sanitized_string_display() {
        let s = SanitizedString::from("hello");
        assert_eq!(s.to_string(), "hello");
    }

    #[test]
    fn sanitized_string_partial_eq_str() {
        let s = SanitizedString::from("hello");
        assert!(s == *"hello");
        assert!(!(s == *"world"));
    }

    #[test]
    fn value_string_sanitizes_control_chars() {
        assert_eq!(
            Value::from("\x1b[31mred\x1b[0m"),
            Value::String("red".into())
        );
        assert_eq!(
            Value::from("with\nnewline"),
            Value::String("withnewline".into())
        );
        assert_eq!(
            Value::from("clean string"),
            Value::String("clean string".into())
        );
    }

    #[test]
    fn value_from_conversions() {
        assert_eq!(Value::from("hello"), Value::String("hello".into()));
        assert_eq!(
            Value::from(String::from("owned")),
            Value::String("owned".into())
        );
        assert_eq!(Value::from(42i64), Value::Int(42));
        assert_eq!(Value::from(42i32), Value::Int(42));
        assert_eq!(Value::from(42u32), Value::Int(42));
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(2.72f64), Value::Float(2.72));
    }

    #[test]
    fn value_from_u64_within_range() {
        assert_eq!(Value::from(42u64), Value::Int(42));
    }

    #[test]
    fn value_from_u64_overflow_becomes_string() {
        let val = Value::from(u64::MAX);
        assert_eq!(val, Value::String(u64::MAX.to_string().into()));
    }

    #[test]
    fn value_from_usize_overflow_becomes_string() {
        let val = Value::from(usize::MAX);
        assert_eq!(val, Value::String(usize::MAX.to_string().into()));
    }

    #[test]
    fn value_from_vec() {
        let val = Value::from(vec!["a", "b", "c"]);
        match val {
            Value::List(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], Scalar::String("a".into()));
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn value_display() {
        assert_eq!(Value::String("hello".into()).to_string(), "hello");
        assert_eq!(Value::Int(42).to_string(), "42");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::Float(2.72).to_string(), "2.72");
        assert_eq!(
            Value::List(vec![Scalar::from("a"), Scalar::from("b")]).to_string(),
            "a, b"
        );
        assert_eq!(Value::List(vec![]).to_string(), "");
        assert_eq!(Value::Redacted.to_string(), "[REDACTED]");
    }

    #[test]
    fn scalar_display() {
        assert_eq!(Scalar::String("hello".into()).to_string(), "hello");
        assert_eq!(Scalar::Int(42).to_string(), "42");
        assert_eq!(Scalar::Bool(true).to_string(), "true");
        assert_eq!(Scalar::Redacted.to_string(), "[REDACTED]");
    }

    #[test]
    fn scalar_from_conversions() {
        assert_eq!(Scalar::from("hello"), Scalar::String("hello".into()));
        assert_eq!(Scalar::from(42i64), Scalar::Int(42));
        assert_eq!(Scalar::from(true), Scalar::Bool(true));
        assert_eq!(Scalar::from(2.72f64), Scalar::Float(2.72));
    }

    #[test]
    fn value_list_holds_only_scalars() {
        // Value::List(Vec<Scalar>) prevents nesting at the type level.
        // This test verifies the flat-list contract at runtime.
        let list = Value::from(vec!["a", "b"]);
        match list {
            Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], Scalar::String(s) if s == "a"));
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn message_strips_full_ansi_sequences() {
        let event = LogEvent::new(
            Level::Warn,
            Source::turbo(Subsystem::Cache),
            "hello\x1b[31mworld\x00".to_string(),
        );
        assert_eq!(event.message, "helloworld");
    }

    #[test]
    fn message_strips_clear_screen_sequence() {
        let event = LogEvent::new(
            Level::Warn,
            Source::turbo(Subsystem::Cache),
            "before\x1b[2Jafter".to_string(),
        );
        assert_eq!(event.message, "beforeafter");
    }

    #[test]
    fn message_preserves_newlines() {
        let event = LogEvent::new(
            Level::Info,
            Source::turbo(Subsystem::Cache),
            "line 1\nline 2".to_string(),
        );
        assert_eq!(event.message, "line 1\nline 2");
    }

    #[test]
    fn serialization_omits_empty_fields() {
        let event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "msg");
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fields").is_none());
    }

    #[test]
    fn serialization_fields_as_map() {
        let mut event = LogEvent::new(Level::Error, Source::task("web#build"), "failed");
        event.fields.push(("code", Value::from(137i64)));
        event.fields.push(("signal", Value::from("SIGKILL")));
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["fields"]["code"], 137);
        assert_eq!(parsed["fields"]["signal"], "SIGKILL");
    }

    #[test]
    fn serialization_redacted_field_is_null() {
        let mut event = LogEvent::new(Level::Info, Source::turbo(Subsystem::Cache), "token used");
        event.fields.push(("token", Value::Redacted));
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["fields"]["token"].is_null());
    }

    #[test]
    fn serialization_timestamp_is_epoch_millis() {
        let event = LogEvent::new(Level::Info, Source::turbo(Subsystem::Cache), "msg");
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["timestamp"].is_u64());
    }

    #[test]
    fn serialization_level_is_uppercase() {
        let event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "msg");
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["level"], "WARN");
    }

    #[test]
    fn with_timestamp_uses_provided_time() {
        let ts = SystemTime::UNIX_EPOCH;
        let event =
            LogEvent::with_timestamp(Level::Info, Source::turbo(Subsystem::Cache), "msg", ts);
        assert_eq!(event.timestamp, ts);
    }

    #[test]
    fn strip_control_chars_fast_path() {
        let input = "no control chars here";
        assert!(matches!(
            strip_control_chars(input, false),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn strip_control_chars_preserves_newlines_when_requested() {
        assert_eq!(strip_control_chars("a\nb", true).as_ref(), "a\nb");
        assert_eq!(strip_control_chars("a\nb", false).as_ref(), "ab");
    }

    #[test]
    fn strip_c1_csi_consumes_parameter_bytes() {
        assert_eq!(
            strip_control_chars("\u{9b}31mred\u{9b}0m", false).as_ref(),
            "red"
        );
    }

    #[test]
    fn strip_c1_csi_clear_screen() {
        assert_eq!(
            strip_control_chars("before\u{9b}2Jafter", false).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn source_task_strips_c1_csi() {
        let source = Source::task("evil\u{9b}31mtask");
        match &source {
            Source::Task(id) => assert_eq!(id.as_ref(), "eviltask"),
            _ => panic!("expected Task"),
        }
    }

    #[test]
    fn message_strips_c1_csi() {
        let event = LogEvent::new(
            Level::Warn,
            Source::turbo(Subsystem::Cache),
            "\u{9b}31mred\u{9b}0m text",
        );
        assert_eq!(event.message, "red text");
    }

    #[test]
    fn value_string_strips_c1_csi() {
        assert_eq!(
            Value::from("\u{9b}31mred\u{9b}0m"),
            Value::String("red".into())
        );
    }

    #[test]
    fn strip_csi_with_backtick_terminator() {
        assert_eq!(
            strip_control_chars("before\x1b[5`after", true).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn strip_csi_with_curly_brace_terminator() {
        assert_eq!(
            strip_control_chars("before\x1b[0{after", true).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn strip_csi_with_pipe_terminator() {
        assert_eq!(
            strip_control_chars("before\x1b[0|after", true).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn strip_fp_escape_cursor_save() {
        // ESC 7 = DECSC (cursor save), Fp range 0x30-0x3F
        assert_eq!(
            strip_control_chars("before\x1b7after", true).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn strip_fs_escape_ris() {
        // ESC c = RIS (Reset to Initial State), Fs range 0x60-0x7E
        assert_eq!(
            strip_control_chars("before\x1bcafter", true).as_ref(),
            "beforeafter"
        );
    }

    #[test]
    fn sanitized_string_from_trusted_bypasses_sanitization() {
        let raw = SanitizedString::from_trusted("\x1b[31mred\x1b[0m".to_string());
        assert!(raw.as_str().contains("\x1b[31m"));
        let sanitized = SanitizedString::from("\x1b[31mred\x1b[0m");
        assert_eq!(sanitized.as_str(), "red");
    }

    #[test]
    fn accessors_return_expected_values() {
        let ts = SystemTime::UNIX_EPOCH;
        let mut event =
            LogEvent::with_timestamp(Level::Warn, Source::turbo(Subsystem::Cache), "msg", ts);
        event.push_field("key", Value::from("val"));
        assert_eq!(event.level(), Level::Warn);
        assert_eq!(event.source(), &Source::turbo(Subsystem::Cache));
        assert_eq!(event.message(), "msg");
        assert_eq!(event.fields().len(), 1);
        assert_eq!(event.fields()[0].0, "key");
        assert_eq!(event.timestamp(), ts);
    }

    #[test]
    fn strip_del_character() {
        assert_eq!(
            strip_control_chars("hello\x7Fworld", true).as_ref(),
            "helloworld"
        );
    }

    #[test]
    fn scalar_to_value_conversion() {
        let scalar = Scalar::from("hello");
        let value = Value::from(scalar);
        assert_eq!(value, Value::String("hello".into()));

        let scalar = Scalar::from(42i64);
        let value = Value::from(scalar);
        assert_eq!(value, Value::Int(42));
    }
}
