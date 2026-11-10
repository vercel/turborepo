//! The conformance checks.

use std::{
    fmt,
    hint::black_box,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{PoisonError, RwLock, RwLockReadGuard},
    time::{Duration, Instant},
};

use turborepo_trace_core::{
    ExtractCx, FileId, Language, PathInterner, RawRef, RefSink, Span, TraceOptions, Uncertainty,
};

use crate::{
    Config, Wrapper,
    rng::{XorShift, fnv1a},
};

/// A failed conformance check: which check, what went wrong, and the offending
/// input.
#[derive(Clone, Debug)]
pub struct Failure {
    /// The check that failed, such as `"spans"`.
    pub check: &'static str,
    /// Human-readable description.
    pub message: String,
    /// The input that triggered the failure, if any.
    pub input: Option<Vec<u8>>,
}

impl Failure {
    fn new(check: &'static str, message: impl Into<String>) -> Self {
        Failure {
            check,
            message: message.into(),
            input: None,
        }
    }

    fn with_input(mut self, input: &[u8]) -> Self {
        self.input = Some(input.to_vec());
        self
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conformance check `{}` failed: {}",
            self.check, self.message
        )?;
        if let Some(input) = &self.input {
            const MAX: usize = 4096;
            let shown = &input[..input.len().min(MAX)];
            write!(
                f,
                "\ninput ({} bytes): b\"{}\"",
                input.len(),
                shown.escape_ascii()
            )?;
            if input.len() > MAX {
                write!(f, "... (truncated)")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for Failure {}

/// Serializes the timing-sensitive linear-time check against the other
/// (CPU-heavy) checks running on parallel test threads of the same process.
static TIMING: RwLock<()> = RwLock::new(());

fn shared_lock() -> RwLockReadGuard<'static, ()> {
    TIMING.read().unwrap_or_else(PoisonError::into_inner)
}

/// Extraction output: references and diagnostics.
type Output<'s> = (Vec<RawRef<'s>>, Vec<Uncertainty>);

/// Runs a language's extractor with a stable virtual context.
struct Harness<'a> {
    lang: &'a dyn Language,
    path: PathBuf,
    file: FileId,
    options: TraceOptions,
}

impl<'a> Harness<'a> {
    fn new(lang: &'a dyn Language, path: &str) -> Self {
        let path = PathBuf::from(path);
        let file = PathInterner::new().intern(&path);
        let options = TraceOptions {
            base: path.parent().unwrap_or(Path::new("/")).to_path_buf(),
            cwd: path.parent().unwrap_or(Path::new("/")).to_path_buf(),
            ..TraceOptions::default()
        };
        Harness {
            lang,
            path,
            file,
            options,
        }
    }

    /// Extracts `src`, turning a panic into an error message.
    fn extract<'s>(&self, src: &'s [u8]) -> Result<Output<'s>, String> {
        let cx = ExtractCx {
            file: self.file,
            path: &self.path,
            options: &self.options,
        };
        catch_unwind(AssertUnwindSafe(|| {
            let mut sink = RefSink::new();
            self.lang.extract(src, &cx, &mut sink);
            sink.into_parts()
        }))
        .map_err(|payload| {
            if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_owned()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "non-string panic payload".to_owned()
            }
        })
    }

    /// Extracts `src` and validates panics and spans.
    fn validated<'s>(&self, check: &'static str, src: &'s [u8]) -> Result<Output<'s>, Failure> {
        let out = self.extract(src).map_err(|msg| {
            Failure::new(check, format!("extract panicked: {msg}")).with_input(src)
        })?;
        validate_spans(src, &out).map_err(|msg| Failure::new(check, msg).with_input(src))?;
        Ok(out)
    }

    fn count(&self, check: &'static str, src: &[u8]) -> Result<usize, Failure> {
        Ok(self.validated(check, src)?.0.len())
    }
}

fn describe_refs(refs: &[RawRef<'_>]) -> String {
    let items: Vec<String> = refs
        .iter()
        .map(|r| format!("{} `{}` @{}..{}", r.kind, r.spec, r.span.start, r.span.end))
        .collect();
    format!("[{}]", items.join(", "))
}

/// Checks every span in an extraction result.
fn validate_spans(src: &[u8], (refs, diags): &Output<'_>) -> Result<(), String> {
    let utf8 = std::str::from_utf8(src).ok();
    let check = |what: &str, span: Span| -> Result<(), String> {
        let (start, end) = (span.start as usize, span.end as usize);
        if start > end {
            return Err(format!("{what} span {start}..{end} has start > end"));
        }
        if end > src.len() {
            return Err(format!(
                "{what} span {start}..{end} is out of bounds (input is {} bytes)",
                src.len()
            ));
        }
        if let Some(s) = utf8
            && (!s.is_char_boundary(start) || !s.is_char_boundary(end))
        {
            return Err(format!(
                "{what} span {start}..{end} is not on UTF-8 char boundaries"
            ));
        }
        Ok(())
    };
    for r in refs {
        check("reference", r.span)?;
        if let Some(u) = &r.uncertainty {
            check("reference uncertainty", u.span)?;
        }
    }
    for d in diags {
        check("diagnostic", d.span)?;
    }
    Ok(())
}

/// Validates one input the way a fuzz target should: no panic, valid spans, and
/// identical output when extracted twice (from two different buffers).
pub fn check_input(lang: &dyn Language, path: &str, src: &[u8]) -> Result<(), Failure> {
    let h = Harness::new(lang, path);
    determinism_one(&h, src)
}

fn determinism_one(h: &Harness<'_>, src: &[u8]) -> Result<(), Failure> {
    const CHECK: &str = "deterministic";
    let first = h.validated(CHECK, src)?;
    let copy = src.to_vec();
    let second = h.validated(CHECK, &copy)?;
    if first.0 != second.0 {
        return Err(Failure::new(
            CHECK,
            format!(
                "references differ between two runs:\n  first:  {}\n  second: {}",
                describe_refs(&first.0),
                describe_refs(&second.0)
            ),
        )
        .with_input(src));
    }
    if first.1 != second.1 {
        return Err(Failure::new(
            CHECK,
            format!(
                "diagnostics differ between two runs:\n  first:  {:?}\n  second: {:?}",
                first.1, second.1
            ),
        )
        .with_input(src));
    }
    Ok(())
}

/// All wrappers with a label.
fn wrappers(cfg: &Config) -> impl Iterator<Item = (&'static str, &Wrapper)> {
    cfg.comments
        .iter()
        .map(|w| ("comment", w))
        .chain(cfg.strings.iter().map(|w| ("string", w)))
}

/// Valid-ish inputs built from the config: corpus, samples, wrapped samples.
fn seeds(cfg: &Config) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = cfg.corpus.iter().map(|c| c.to_vec()).collect();
    for s in cfg.samples {
        out.push(s.as_bytes().to_vec());
    }
    if !cfg.samples.is_empty() {
        out.push(cfg.samples.join(cfg.separator).into_bytes());
    }
    for (_, w) in wrappers(cfg) {
        for s in cfg.samples {
            let (wrapped, _) = w.wrap(s);
            out.push(contextual(cfg, s, &wrapped));
            out.push(wrapped);
        }
    }
    out
}

/// `sample sep wrapped sep sample sep`.
fn contextual(cfg: &Config, sample: &str, wrapped: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(sample.as_bytes());
    v.extend_from_slice(cfg.separator.as_bytes());
    v.extend_from_slice(wrapped);
    v.extend_from_slice(cfg.separator.as_bytes());
    v.extend_from_slice(sample.as_bytes());
    v.extend_from_slice(cfg.separator.as_bytes());
    v
}

/// Check (a): references inside comments and strings produce no references, and
/// the lexer resynchronizes after the construct.
pub fn check_comments_and_strings(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    let _guard = shared_lock();
    const CHECK: &str = "comments_and_strings";
    let h = Harness::new(lang, cfg.path);
    if cfg.samples.is_empty() {
        return Err(Failure::new(CHECK, "Config::samples is empty"));
    }
    if cfg.comments.is_empty() && cfg.strings.is_empty() {
        return Err(Failure::new(
            CHECK,
            "Config::comments and Config::strings are both empty",
        ));
    }
    let mut sample_counts = Vec::new();
    for s in cfg.samples {
        let k = h.count(CHECK, s.as_bytes())?;
        if k == 0 {
            return Err(Failure::new(
                CHECK,
                format!("sample {s:?} produces no references on its own"),
            )
            .with_input(s.as_bytes()));
        }
        sample_counts.push(k);
    }
    for (label, w) in wrappers(cfg) {
        let (empty, _) = w.wrap("");
        let base = h.count(CHECK, &empty)?;
        for (s, &k) in cfg.samples.iter().zip(&sample_counts) {
            let (wrapped, inserted) = w.wrap(s);
            let (refs, _) = h.validated(CHECK, &wrapped)?;
            let inside: Vec<_> = refs
                .iter()
                .filter(|r| {
                    let (a, b) = (r.span.start as usize, r.span.end as usize);
                    a >= inserted.start && b <= inserted.end
                })
                .cloned()
                .collect();
            if refs.len() != base || !inside.is_empty() {
                return Err(Failure::new(
                    CHECK,
                    format!(
                        "sample {s:?} inside {label} {:?}...{:?} produced references (expected \
                         {base} like the empty {label}, got {}): {}",
                        w.open,
                        w.close,
                        refs.len(),
                        describe_refs(&refs)
                    ),
                )
                .with_input(&wrapped));
            }
            let ctx = contextual(cfg, s, &wrapped);
            let (refs, _) = h.validated(CHECK, &ctx)?;
            if refs.len() != 2 * k + base {
                return Err(Failure::new(
                    CHECK,
                    format!(
                        "sample {s:?} before and after a {label} {:?}...{:?} containing it: \
                         expected {} references, got {}: {} (the {label} leaked or the lexer did \
                         not resynchronize)",
                        w.open,
                        w.close,
                        2 * k + base,
                        refs.len(),
                        describe_refs(&refs)
                    ),
                )
                .with_input(&ctx));
            }
        }
    }
    Ok(())
}

/// Byte sequences that are invalid or tricky UTF-8.
const BAD_UTF8: &[&[u8]] = &[
    b"\xff",
    b"\xfe\xff",
    b"\x80",
    b"\xc3",
    b"\xc0\xaf",
    b"\xe2\x82",
    b"\xed\xa0\x80",
    b"\xf4\x90\x80\x80",
    b"\xf0\x9f\x98",
    b"\x00",
    "é".as_bytes(),
    "😀".as_bytes(),
    "\u{feff}".as_bytes(),
    "\u{2028}".as_bytes(),
];

/// Every prefix of `src` (or ~2048 evenly spaced ones for long inputs).
fn truncations(src: &[u8]) -> impl Iterator<Item = &[u8]> {
    let step = (src.len() / 2048).max(1);
    (0..src.len()).step_by(step).map(move |n| &src[..n])
}

/// Check (b): spans are in bounds, ordered, and on char boundaries for UTF-8
/// input. Runs on the seeds, all their truncations, and seeds with invalid
/// UTF-8 injected.
pub fn check_spans(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    let _guard = shared_lock();
    const CHECK: &str = "spans";
    let h = Harness::new(lang, cfg.path);
    let mut rng = XorShift::new(seed(lang, cfg) ^ 0x5350_414e);
    for s in seeds(cfg) {
        h.validated(CHECK, &s)?;
        for t in truncations(&s) {
            h.validated(CHECK, t)?;
        }
        for _ in 0..8 {
            let mut m = s.clone();
            let at = rng.below(m.len() + 1);
            let bad = rng.pick(BAD_UTF8).copied().unwrap_or(b"\xff");
            m.splice(at..at, bad.iter().copied());
            h.validated(CHECK, &m)?;
        }
    }
    Ok(())
}

fn seed(lang: &dyn Language, cfg: &Config) -> u64 {
    if let Some(s) = turborepo_trace_core::debug::var("TESTKIT_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return s;
    }
    fnv1a(lang.id().0.as_bytes()) ^ fnv1a(cfg.path.as_bytes()).rotate_left(17)
}

fn iterations(cfg: &Config) -> usize {
    turborepo_trace_core::debug::var("TESTKIT_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(cfg.fuzz_iterations)
}

/// Generates mutated and random inputs deterministically.
struct Mutator {
    rng: XorShift,
    seeds: Vec<Vec<u8>>,
    dict: Vec<Vec<u8>>,
    alphabet: Vec<u8>,
}

impl Mutator {
    const MAX_LEN: usize = 64 * 1024;

    fn new(cfg: &Config, seed: u64) -> Self {
        let seeds = seeds(cfg);
        let mut dict: Vec<Vec<u8>> = cfg.tokens.iter().map(|t| t.to_vec()).collect();
        for (_, w) in wrappers(cfg) {
            dict.push(w.open.as_bytes().to_vec());
            dict.push(w.close.as_bytes().to_vec());
        }
        for s in cfg.samples {
            dict.push(s.as_bytes().to_vec());
        }
        dict.push(cfg.separator.as_bytes().to_vec());
        dict.extend(BAD_UTF8.iter().map(|b| b.to_vec()));
        let mut seen = [false; 256];
        for b in seeds.iter().chain(&dict).flatten() {
            seen[*b as usize] = true;
        }
        for b in b"\\\"'`/*#\n\r\t ({[]})$" {
            seen[*b as usize] = true;
        }
        let alphabet = (0..=255u8).filter(|b| seen[*b as usize]).collect();
        Mutator {
            rng: XorShift::new(seed),
            seeds,
            dict,
            alphabet,
        }
    }

    fn random_bytes(&mut self) -> Vec<u8> {
        let len = self.rng.below(512);
        (0..len).map(|_| self.rng.byte()).collect()
    }

    fn random_alphabet(&mut self) -> Vec<u8> {
        let len = self.rng.below(512);
        (0..len)
            .map(|_| *self.rng.pick(&self.alphabet).unwrap_or(&b' '))
            .collect()
    }

    fn mutated(&mut self) -> Vec<u8> {
        let mut v = self.rng.pick(&self.seeds).cloned().unwrap_or_default();
        let ops = 1 + self.rng.below(6);
        for _ in 0..ops {
            let len = v.len();
            match self.rng.below(9) {
                0 if len > 0 => {
                    let i = self.rng.below(len);
                    v[i] ^= 1 << self.rng.below(8);
                }
                1 if len > 0 => {
                    let i = self.rng.below(len);
                    v[i] = self.rng.byte();
                }
                2 => {
                    let i = self.rng.below(len + 1);
                    let b = self.rng.byte();
                    v.insert(i, b);
                }
                3 => {
                    let i = self.rng.below(len + 1);
                    let tok = self.rng.pick(&self.dict).cloned().unwrap_or_default();
                    v.splice(i..i, tok);
                }
                4 if len > 0 => {
                    let i = self.rng.below(len);
                    let n = 1 + self.rng.below(16);
                    v.drain(i..(i + n).min(len));
                }
                5 if len > 0 => {
                    let i = self.rng.below(len);
                    let n = 1 + self.rng.below(64);
                    let chunk = v[i..(i + n).min(len)].to_vec();
                    let at = self.rng.below(v.len() + 1);
                    v.splice(at..at, chunk);
                }
                6 => {
                    let other = self.rng.pick(&self.seeds).cloned().unwrap_or_default();
                    let a = self.rng.below(len + 1);
                    let b = self.rng.below(other.len() + 1);
                    v.truncate(a);
                    v.extend_from_slice(&other[b..]);
                }
                7 => {
                    let n = self.rng.below(len + 1);
                    v.truncate(n);
                }
                _ => {
                    let i = self.rng.below(len + 1);
                    let b = *self.rng.pick(&self.alphabet).unwrap_or(&b'\n');
                    v.insert(i, b);
                }
            }
        }
        v.truncate(Self::MAX_LEN);
        v
    }
}

/// Check (c): arbitrary bytes, invalid UTF-8, truncated and mutated inputs
/// never panic (and keep spans valid).
pub fn check_no_panic(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    let _guard = shared_lock();
    const CHECK: &str = "no_panic";
    let h = Harness::new(lang, cfg.path);
    let seed = seed(lang, cfg);
    let mut m = Mutator::new(cfg, seed);
    let with_seed = |f: Failure| Failure {
        message: format!("{} (TURBO_TRACE_DEBUG_TESTKIT_SEED={seed})", f.message),
        ..f
    };
    // Edge cases first.
    let fixed: &[&[u8]] = &[
        b"", b"\n", b"\r\n", b"\0", b"\\", b"\"", b"'", b"`", b"/", b"#",
    ];
    for src in fixed.iter().copied().chain(BAD_UTF8.iter().copied()) {
        h.validated(CHECK, src).map_err(with_seed)?;
    }
    for (_, w) in wrappers(cfg) {
        // Unterminated constructs.
        h.validated(CHECK, w.open.as_bytes()).map_err(with_seed)?;
        h.validated(CHECK, w.close.as_bytes()).map_err(with_seed)?;
    }
    let n = iterations(cfg);
    for i in 0..n {
        let src = match i % 4 {
            0 => m.random_bytes(),
            1 => m.random_alphabet(),
            _ => m.mutated(),
        };
        h.validated(CHECK, &src).map_err(with_seed)?;
    }
    Ok(())
}

/// Check (e): the same input extracted twice (from different buffers) yields
/// identical references and diagnostics.
pub fn check_deterministic(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    let _guard = shared_lock();
    let h = Harness::new(lang, cfg.path);
    let seeds = seeds(cfg);
    for s in &seeds {
        determinism_one(&h, s)?;
    }
    let mut m = Mutator::new(cfg, seed(lang, cfg) ^ 0xDE7E_4313);
    for _ in 0..(iterations(cfg) / 10).max(50) {
        determinism_one(&h, &m.mutated())?;
    }
    Ok(())
}

/// Inputs repeated for the linear-time check: realistic code plus pathological
/// shapes (unterminated constructs, statements without separators).
fn linear_units(cfg: &Config) -> Vec<(String, Vec<u8>)> {
    let mut units = Vec::new();
    let mut corpus = Vec::new();
    for s in seeds(cfg) {
        corpus.extend_from_slice(&s);
        corpus.extend_from_slice(cfg.separator.as_bytes());
    }
    units.push(("corpus".to_owned(), corpus));
    if !cfg.samples.is_empty() {
        units.push((
            "samples without separator".to_owned(),
            cfg.samples.concat().into_bytes(),
        ));
    }
    for (label, w) in wrappers(cfg) {
        let sample = cfg.samples.first().copied().unwrap_or("x");
        let mut unit = w.open.as_bytes().to_vec();
        unit.extend_from_slice(w.escape.apply(sample).as_bytes());
        units.push((format!("unterminated {label} {:?}", w.open), unit));
    }
    units.retain(|(_, u)| !u.is_empty());
    units
}

fn time_once(h: &Harness<'_>, src: &[u8]) -> Result<Duration, Failure> {
    let start = Instant::now();
    let out = h.extract(src).map_err(|msg| {
        Failure::new("linear_time", format!("extract panicked: {msg}"))
            .with_input(&src[..256.min(src.len())])
    })?;
    let elapsed = start.elapsed();
    black_box(out);
    Ok(elapsed)
}

fn best_of(h: &Harness<'_>, src: &[u8], runs: usize) -> Result<Duration, Failure> {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        best = best.min(time_once(h, src)?);
    }
    Ok(best)
}

/// Check (d): extraction time grows linearly. For each unit, `N` repetitions
/// are calibrated to take at least ~2 ms (and at least `linear_min_bytes`),
/// then `time(8N) / time(N)` must stay below `linear_max_ratio`. Small and big
/// runs are interleaved for up to 12 trials (or 20 s) and the ratio uses the
/// minimum time of each, so transient noise does not fail it.
pub fn check_linear_time(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    let _exclusive = TIMING.write().unwrap_or_else(PoisonError::into_inner);
    const CHECK: &str = "linear_time";
    const MIN_TIME: Duration = Duration::from_millis(2);
    const MAX_SMALL: usize = 4 * 1024 * 1024;
    let h = Harness::new(lang, cfg.path);
    for (name, unit) in linear_units(cfg) {
        let mut n = cfg.linear_min_bytes.div_ceil(unit.len()).max(1);
        let mut small = unit.repeat(n);
        while best_of(&h, &small, 3)? < MIN_TIME && small.len() < MAX_SMALL {
            n *= 2;
            small = unit.repeat(n);
        }
        let big = unit.repeat(8 * n);
        // Interleave small and big runs and keep the minimum of each across all trials,
        // so a burst of noise (other processes, a migration to an efficiency
        // core) has to hit every big run to cause a failure.
        let deadline = Instant::now() + Duration::from_secs(20);
        let (mut min_small, mut min_big) = (Duration::MAX, Duration::MAX);
        let mut ratio = f64::INFINITY;
        let mut ok = false;
        for trial in 0..12 {
            min_small = min_small.min(best_of(&h, &small, 2)?);
            min_big = min_big.min(time_once(&h, &big)?);
            ratio = min_big.as_secs_f64() / min_small.as_secs_f64().max(1e-9);
            if ratio <= cfg.linear_max_ratio {
                ok = true;
                break;
            }
            // Clearly super-linear (quadratic scores ~64x): do not keep re-measuring.
            if (trial >= 5 && ratio > 3.0 * cfg.linear_max_ratio) || Instant::now() > deadline {
                break;
            }
        }
        if !ok {
            return Err(Failure::new(
                CHECK,
                format!(
                    "extraction looks super-linear on {name}: {} bytes took {min_small:?}, {} \
                     bytes took {min_big:?}, ratio {ratio:.1}x (limit {:.1}x). Unit shown below.",
                    small.len(),
                    big.len(),
                    cfg.linear_max_ratio
                ),
            )
            .with_input(&unit));
        }
    }
    Ok(())
}

/// Runs every check and returns the first failure.
pub fn run_all(lang: &dyn Language, cfg: &Config) -> Result<(), Failure> {
    check_comments_and_strings(lang, cfg)?;
    check_spans(lang, cfg)?;
    check_no_panic(lang, cfg)?;
    check_deterministic(lang, cfg)?;
    check_linear_time(lang, cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! The checks must catch broken languages. Each test language is
    //! deliberately wrong in one way.

    use std::sync::atomic::{AtomicU32, Ordering};

    use turborepo_trace_core::{
        ProjectCx, RefKind, Resolution, ResolveCx, Resolver, Specifier, UncertaintyKind,
    };

    use super::*;
    use crate::{Escape, Wrapper};

    struct NoResolver;
    impl Resolver for NoResolver {
        fn resolve(&self, r: &RawRef<'_>, _: FileId, _: &ResolveCx<'_>) -> Resolution {
            Resolution::unresolved(Uncertainty::new(
                UncertaintyKind::UnsupportedSyntax,
                r.span,
                "test",
            ))
        }
    }

    /// A toy language: lines `use "x"`, `#` comments, `say "..."` strings.
    #[derive(Clone, Copy)]
    enum Toy {
        Good,
        /// Ignores comments: finds `use` anywhere.
        SeesComments,
        /// Emits a span past the end of input.
        BadSpan,
        /// Panics on a 0xFF byte.
        PanicsOnFF,
        /// Quadratic: rescans from the start on every line.
        Quadratic,
        /// Emits a varying number of refs.
        Flaky,
    }

    static FLAKY: AtomicU32 = AtomicU32::new(0);

    fn find_uses<'s>(src: &'s [u8], line_start: usize, line: &'s [u8], out: &mut RefSink<'s>) {
        let mut i = 0;
        while let Some(off) = line[i..].windows(5).position(|w| w == b"use \"") {
            let q = i + off + 5;
            let Some(len) = line[q..].iter().position(|&b| b == b'"') else {
                return;
            };
            let spec = std::str::from_utf8(&line[q..q + len]).unwrap_or("?");
            let span = Span::from_usize(line_start + q - 1, line_start + q + len + 1);
            debug_assert!(span.end as usize <= src.len());
            out.push(RawRef::new(RefKind::Import, Specifier::literal(spec), span));
            i = q + len + 1;
        }
    }

    impl Language for Toy {
        fn id(&self) -> turborepo_trace_core::LangId {
            turborepo_trace_core::LangId("toy")
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["toy"]
        }
        fn extract<'s>(&self, src: &'s [u8], _: &ExtractCx<'_>, out: &mut RefSink<'s>) {
            let mut pos = 0;
            for line in src.split(|&b| b == b'\n') {
                let start = pos;
                pos += line.len() + 1;
                let trimmed = line.trim_ascii_start();
                match self {
                    Toy::SeesComments => find_uses(src, start, line, out),
                    Toy::Quadratic => {
                        // Rescan everything before this line.
                        black_box(src[..start].iter().filter(|&&b| b == b'"').count());
                        if trimmed.starts_with(b"use \"") {
                            find_uses(src, start, line, out);
                        }
                    }
                    Toy::PanicsOnFF => {
                        assert!(!line.contains(&0xff), "boom");
                        if trimmed.starts_with(b"use \"") {
                            find_uses(src, start, line, out);
                        }
                    }
                    _ => {
                        if trimmed.starts_with(b"use \"") {
                            find_uses(src, start, line, out);
                        }
                    }
                }
            }
            match self {
                Toy::BadSpan if !src.is_empty() => out.push(RawRef::new(
                    RefKind::Import,
                    Specifier::Unknown,
                    Span::from_usize(0, src.len() + 1),
                )),
                Toy::Flaky if FLAKY.fetch_add(1, Ordering::Relaxed).is_multiple_of(2) => out.push(
                    RawRef::new(RefKind::Import, Specifier::Unknown, Span::DUMMY),
                ),
                _ => {}
            }
        }
        fn resolver(&self, _: &ProjectCx<'_>) -> Box<dyn Resolver> {
            Box::new(NoResolver)
        }
    }

    const CONFIG: Config = Config {
        samples: &["use \"./a\""],
        comments: &[Wrapper::new("# ", "\n")],
        strings: &[Wrapper::new("say \"", "\"\n").escape(Escape::Backslash(b"\""))],
        corpus: &[b"use \"./x\"\n# c\nsay \"hi\"\n"],
        fuzz_iterations: 500,
        ..Config::new("/testkit/sample.toy")
    };

    #[test]
    fn good_language_passes() {
        run_all(&Toy::Good, &CONFIG).unwrap();
    }

    #[test]
    fn catches_refs_in_comments() {
        let f = check_comments_and_strings(&Toy::SeesComments, &CONFIG).unwrap_err();
        assert!(f.message.contains("inside comment"), "{f}");
    }

    #[test]
    fn catches_bad_spans() {
        let f = check_spans(&Toy::BadSpan, &CONFIG).unwrap_err();
        assert!(f.message.contains("out of bounds"), "{f}");
    }

    #[test]
    fn catches_panics() {
        let f = check_no_panic(&Toy::PanicsOnFF, &CONFIG).unwrap_err();
        assert!(f.message.contains("panicked: boom"), "{f}");
        assert!(f.to_string().contains("\\xff"), "{f}");
    }

    #[test]
    fn catches_quadratic_time() {
        let cfg = Config {
            linear_min_bytes: 1024,
            ..CONFIG
        };
        let f = check_linear_time(&Toy::Quadratic, &cfg).unwrap_err();
        assert!(f.message.contains("super-linear"), "{f}");
    }

    #[test]
    fn catches_nondeterminism() {
        let f = check_deterministic(&Toy::Flaky, &CONFIG).unwrap_err();
        assert!(f.message.contains("differ"), "{f}");
    }

    #[test]
    fn empty_config_is_rejected() {
        let cfg = Config::new("/x.toy");
        assert!(check_comments_and_strings(&Toy::Good, &cfg).is_err());
    }

    #[test]
    fn escapes() {
        assert_eq!(
            Escape::Backslash(b"\"").apply(r#"a "b" \c"#),
            r#"a \"b\" \\c"#
        );
        assert_eq!(Escape::Double(b'\'').apply("it's"), "it''s");
        assert_eq!(Escape::None.apply("x\"y"), "x\"y");
        let (w, r) = Wrapper::new("<", ">").wrap("ab");
        assert_eq!(w, b"<ab>");
        assert_eq!(r, 1..3);
    }

    #[test]
    fn check_input_accepts_good() {
        check_input(&Toy::Good, "/x.toy", b"use \"a\"\n\xff").unwrap();
        assert!(check_input(&Toy::BadSpan, "/x.toy", b"x").is_err());
    }
}
