//! The mandatory conformance suite for the tracer's language plugins, plus a
//! fixture snapshot helper.
//!
//! Every [`Language`] must pass five checks:
//!
//! | check | what it asserts |
//! |---|---|
//! | [`check_comments_and_strings`] | reference statements placed inside the language's comments and strings produce no references, and the lexer resynchronizes after them |
//! | [`check_spans`] | every span is in bounds, has `start <= end`, and lies on UTF-8 char boundaries when the source is valid UTF-8 |
//! | [`check_no_panic`] | arbitrary bytes, invalid UTF-8, truncated and mutated inputs never panic |
//! | [`check_linear_time`] | extraction time on an input repeated `8N` times is less than ~12x the time for `N` |
//! | [`check_deterministic`] | extracting the same input twice yields identical references |
//!
//! A plugin wires all of them up with one macro call in `tests/conformance.rs`:
//!
//! ```ignore
//! use turborepo_trace_testkit::{Config, Escape, Wrapper, conformance};
//!
//! const CONFIG: Config = Config {
//!     samples: &["import \"./a\""],
//!     comments: &[Wrapper::new("// ", "\n"), Wrapper::new("/* ", " */")],
//!     strings: &[Wrapper::new("x = \"", "\"\n").escape(Escape::Backslash(b"\""))],
//!     corpus: &[include_bytes!("../../turborepo-trace-testkit/fixtures/foo/basic/main.foo")],
//!     ..Config::new("/testkit/sample.foo")
//! };
//!
//! conformance!(turborepo_trace_lang_foo::FooLanguage, CONFIG);
//! ```
//!
//! The fuzz-style checks are deterministic. Set
//! `TURBO_TRACE_DEBUG_TESTKIT_SEED` to explore a different seed and
//! `TURBO_TRACE_DEBUG_TESTKIT_ITERS` to run more (or fewer) mutation rounds.
//! Every failure message includes the offending input (escaped) so it can be
//! turned into a unit test.
//!
//! [`fixture_snapshot!`] traces a fixture directory and compares the JSON
//! output with an [`insta`] snapshot stored next to the calling test.

// Every public item is documented (the workspace lint, kept per crate here).
#![warn(missing_docs)]

mod checks;
mod rng;
mod snapshot;
/// The stub language: a trivial language for end-to-end tests of the tracer.
pub mod stub;

pub use checks::{
    Failure, check_comments_and_strings, check_deterministic, check_input, check_linear_time,
    check_no_panic, check_spans, run_all,
};
#[doc(hidden)]
pub use insta as __insta;
pub use rng::XorShift;
pub use snapshot::{fixture_json, redact_base};
pub use turborepo_trace_core::Language;

/// How the inserted reference statement is escaped inside a [`Wrapper`].
#[derive(Copy, Clone, Debug)]
pub enum Escape {
    /// Insert the statement verbatim (comments, raw strings).
    None,
    /// Prefix `\` and each listed byte with a backslash (most quoted strings).
    Backslash(&'static [u8]),
    /// Double each occurrence of the byte, as in SQL `'it''s'` or C# verbatim
    /// strings.
    Double(u8),
    /// A custom escaping function.
    Custom(fn(&str) -> String),
}

impl Escape {
    /// Applies the escaping to `s`.
    pub fn apply(self, s: &str) -> String {
        match self {
            Escape::None => s.to_owned(),
            Escape::Backslash(set) => {
                let mut out = String::with_capacity(s.len() + 8);
                for ch in s.chars() {
                    if ch == '\\' || (ch.is_ascii() && set.contains(&(ch as u8))) {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                out
            }
            Escape::Double(b) => {
                let mut out = String::with_capacity(s.len() + 8);
                for ch in s.chars() {
                    if ch.is_ascii() && ch as u8 == b {
                        out.push(ch);
                    }
                    out.push(ch);
                }
                out
            }
            Escape::Custom(f) => f(s),
        }
    }
}

/// A comment or string construct of the language: `open`, then the (escaped)
/// content, then `close`. Example: `Wrapper::new("/* ", " */")` or
/// `Wrapper::new("'", "'\n")`.
///
/// `open + close` on its own must be valid code; it may produce references of
/// its own (for example `asset "` / `"` in a language where any string argument
/// is a reference). The check only requires that inserting a reference
/// statement between them adds none.
#[derive(Copy, Clone, Debug)]
pub struct Wrapper {
    /// Text that opens the construct, including any statement it needs around
    /// it.
    pub open: &'static str,
    /// Text that closes it, including a trailing newline if the language needs
    /// one.
    pub close: &'static str,
    /// How the inserted statement is escaped.
    pub escape: Escape,
}

impl Wrapper {
    /// A wrapper whose content is inserted verbatim.
    pub const fn new(open: &'static str, close: &'static str) -> Self {
        Wrapper {
            open,
            close,
            escape: Escape::None,
        }
    }

    /// Sets the escaping applied to the inserted statement.
    pub const fn escape(mut self, escape: Escape) -> Self {
        self.escape = escape;
        self
    }

    /// Builds `open + escape(content) + close` and returns it with the byte
    /// range of the inserted content.
    pub fn wrap(&self, content: &str) -> (Vec<u8>, std::ops::Range<usize>) {
        let inner = self.escape.apply(content);
        let mut out = Vec::with_capacity(self.open.len() + inner.len() + self.close.len());
        out.extend_from_slice(self.open.as_bytes());
        let start = out.len();
        out.extend_from_slice(inner.as_bytes());
        let end = out.len();
        out.extend_from_slice(self.close.as_bytes());
        (out, start..end)
    }
}

/// Configuration for the conformance suite. Build it with struct update syntax
/// on top of [`Config::new`], which is `const`, so the config can live in a
/// `const` item.
#[derive(Copy, Clone, Debug)]
pub struct Config {
    /// Virtual absolute path passed to [`Language::extract`] through
    /// `ExtractCx`. Its extension should be one the language handles.
    pub path: &'static str,
    /// Reference statements. Each must produce at least one reference on its
    /// own.
    pub samples: &'static [&'static str],
    /// Text that separates statements (usually `"\n"`).
    pub separator: &'static str,
    /// Every comment form of the language.
    pub comments: &'static [Wrapper],
    /// Every string form of the language.
    pub strings: &'static [Wrapper],
    /// Realistic sources (fixtures). Seeds for mutation and the linear-time
    /// check.
    pub corpus: &'static [&'static [u8]],
    /// Extra byte sequences used by the mutator (keywords, delimiters).
    pub tokens: &'static [&'static [u8]],
    /// Mutation rounds for [`check_no_panic`];
    /// `TURBO_TRACE_DEBUG_TESTKIT_ITERS` overrides it.
    pub fuzz_iterations: usize,
    /// Maximum allowed `time(8N) / time(N)` ratio. A linear extractor scores
    /// ~8, an `N log N` one ~9, a quadratic one ~64. The default of 16
    /// (growth below `N^1.34`) leaves room for the memory hierarchy: on a
    /// machine with large caches the `N` input fits in a cache that the
    /// `8N` input does not, and a linear extractor then costs up to about 1.6
    /// times more per byte on the big input (measured 12.5 on a hosted CI
    /// runner, minimum of 12 trials).
    pub linear_max_ratio: f64,
    /// Minimum size in bytes of the `N` input in the linear-time check. The 1
    /// MiB default keeps both inputs (and the references they produce)
    /// larger than the per-core caches, so most cache effects do not
    /// masquerade as super-linear growth.
    pub linear_min_bytes: usize,
}

impl Config {
    /// Defaults with no samples, wrappers or corpus.
    pub const fn new(path: &'static str) -> Self {
        Config {
            path,
            samples: &[],
            separator: "\n",
            comments: &[],
            strings: &[],
            corpus: &[],
            tokens: &[],
            fuzz_iterations: 3000,
            linear_max_ratio: 16.0,
            linear_min_bytes: 1024 * 1024,
        }
    }
}

/// Generates one `#[test]` per conformance check for a language.
///
/// ```ignore
/// turborepo_trace_testkit::conformance!(MyLanguage, CONFIG);
/// ```
///
/// The first argument is an expression evaluating to the language value, the
/// second one to a [`Config`]. The tests are generated in a module named
/// `conformance`.
#[macro_export]
macro_rules! conformance {
    ($lang:expr, $config:expr $(,)?) => {
        mod conformance {
            #[allow(unused_imports)]
            use super::*;

            fn run(
                check: fn(&dyn $crate::Language, &$crate::Config) -> Result<(), $crate::Failure>,
            ) {
                let lang = $lang;
                let config: $crate::Config = $config;
                if let Err(failure) = check(&lang, &config) {
                    panic!("{failure}");
                }
            }

            #[test]
            fn comments_and_strings_produce_no_refs() {
                run($crate::check_comments_and_strings);
            }

            #[test]
            fn spans_are_valid() {
                run($crate::check_spans);
            }

            #[test]
            fn arbitrary_input_never_panics() {
                run($crate::check_no_panic);
            }

            #[test]
            fn extraction_is_linear() {
                run($crate::check_linear_time);
            }

            #[test]
            fn output_is_deterministic() {
                run($crate::check_deterministic);
            }
        }
    };
}

/// Traces `entries` (relative to `fixture_dir`) and asserts the JSON output
/// against an [`insta`] snapshot named `name`, stored in the calling crate's
/// `snapshots/` directory.
///
/// The second argument is a `TracerBuilder` with the languages (and any
/// options) configured; the base is set to the fixture directory. The absolute
/// base path is redacted as `<fixture>` so snapshots are machine independent.
///
/// ```ignore
/// turborepo_trace_testkit::fixture_snapshot!(
///     "basic_main",
///     turborepo_trace_core::Tracer::builder().language(StubLanguage),
///     concat!(env!("CARGO_MANIFEST_DIR"), "/../turborepo-trace-testkit/fixtures/stub/basic"),
///     &["main.stub"],
/// );
/// ```
///
/// Accept new or changed snapshots with `INSTA_UPDATE=always cargo test` or
/// `cargo insta review`.
#[macro_export]
macro_rules! fixture_snapshot {
    ($name:expr, $builder:expr, $fixture_dir:expr, $entries:expr $(,)?) => {{
        let json = $crate::fixture_json($builder, $fixture_dir, $entries);
        $crate::__insta::assert_snapshot!($name, json);
    }};
}
