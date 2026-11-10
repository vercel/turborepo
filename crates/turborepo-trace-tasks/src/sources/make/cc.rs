//! C/C++ compiler and archiver commands, as found in Makefile recipes.
//!
//! `cc`/`gcc`/`clang`/`c++`/`g++`/`clang++` (and cross or versioned variants
//! such as `arm-none-eabi-gcc`, `gcc-13`): source and object arguments are
//! inputs, `-o` is the output, and the `#include "…"` / `#include <…>` closure
//! of every source is followed through the source's directory, `-iquote` and
//! `-I` directories (files inside the repository only; system headers are
//! toolchain). A computed `#include MACRO` or a response file (`@args`) is not
//! modelled (partial).
//!
//! `ar rcs lib.a a.o b.o`: members are inputs, the archive is the output.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, TaskSpec},
};

/// Maximum number of headers followed per command.
const MAX_HEADERS: usize = 4_000;
/// Maximum bytes read per source or header.
const MAX_SOURCE: usize = 2 * 1024 * 1024;

const COMPILERS: &[&str] = &[
    "cc", "gcc", "clang", "c++", "g++", "clang++", "tcc", "icc", "icx",
];

const SOURCE_EXTS: &[&str] = &[
    "c", "cc", "cpp", "cxx", "c++", "C", "m", "mm", "s", "S", "sx", "i", "ii", "h", "hh", "hpp",
    "hxx",
];

/// Whether `name` is a C/C++ compiler driver.
pub fn is_compiler(name: &str) -> bool {
    COMPILERS.contains(&name)
        || ["gcc-", "clang-", "g++-", "clang++-"].iter().any(|p| {
            name.starts_with(p)
                && name[p.len()..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'.')
        })
        || ["-gcc", "-g++", "-clang", "-clang++", "-cc", "-c++"]
            .iter()
            .any(|s| name.ends_with(s) && name.len() > s.len())
}

/// C/C++ compiler drivers.
pub struct Cc;

impl Recognizer for Cc {
    fn id(&self) -> &'static str {
        "cc"
    }

    fn programs(&self) -> &'static [&'static str] {
        COMPILERS
    }

    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name().is_some_and(is_compiler)
    }

    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("cc").to_string();
        out.tool("system", prog.as_str());
        let words: Vec<&str> = cmd.args().collect();
        let mut quote_dirs: Vec<PathBuf> = Vec::new();
        let mut inc_dirs: Vec<PathBuf> = Vec::new();
        let mut sources: Vec<PathBuf> = Vec::new();
        let mut forced: Vec<PathBuf> = Vec::new();
        let mut partial = false;
        let mut i = 0;
        let value_opts = [
            "-x",
            "-D",
            "-U",
            "-MT",
            "-MQ",
            "-Xlinker",
            "-Xassembler",
            "-Xpreprocessor",
            "-arch",
            "-target",
            "-isysroot",
            "-framework",
            "-u",
            "-e",
            "-z",
            "-B",
            "-G",
            "-std",
            "-wrapper",
            "--param",
            "-Xclang",
            "-mllvm",
            "-l",
            "-L",
            "-F",
        ];
        while i < words.len() {
            let w = words[i];
            i += 1;
            let next = |i: &mut usize| -> Option<&str> {
                let v = words.get(*i).copied();
                *i += 1;
                v
            };
            match w {
                "-o" => {
                    if let Some(o) = next(&mut i) {
                        out.output(cmd.path(o));
                    }
                }
                "-MF" => {
                    if let Some(o) = next(&mut i) {
                        out.output(cmd.path(o));
                    }
                }
                "-I" | "-isystem" | "-idirafter" => {
                    if let Some(d) = next(&mut i) {
                        inc_dirs.push(cmd.path(d));
                    }
                }
                "-iquote" => {
                    if let Some(d) = next(&mut i) {
                        quote_dirs.push(cmd.path(d));
                    }
                }
                "-include" | "-imacros" => {
                    if let Some(f) = next(&mut i) {
                        forced.push(cmd.path(f));
                    }
                }
                "-T" => {
                    if let Some(f) = next(&mut i) {
                        out.input(cmd.path(f));
                    }
                }
                _ if value_opts.contains(&w) => {
                    let _ = next(&mut i);
                }
                _ if w.starts_with("-I") => inc_dirs.push(cmd.path(&w[2..])),
                _ if w.starts_with("-isystem") => inc_dirs.push(cmd.path(&w[8..])),
                _ if w.starts_with("-iquote") => quote_dirs.push(cmd.path(&w[7..])),
                _ if w.starts_with("-o") && w.len() > 2 => out.output(cmd.path(&w[2..])),
                _ if w.starts_with("--sysroot") || w.starts_with('-') => {}
                _ if w.starts_with('@') => {
                    out.input(cmd.path(&w[1..]));
                    out.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("{prog} response file {w}"),
                    );
                    partial = true;
                }
                _ => {
                    let p = cmd.path(w);
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if SOURCE_EXTS.contains(&ext) {
                        sources.push(p.clone());
                    }
                    out.input(p);
                }
            }
        }
        for f in &forced {
            out.input(f.clone());
        }
        let roots: Vec<PathBuf> = sources.iter().chain(&forced).cloned().collect();
        partial |= include_closure(&roots, &quote_dirs, &inc_dirs, cx.repo_root, out);
        if partial {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// Follows `#include`s from `roots`: every header found inside `repo` is an
/// input; a quoted include found nowhere is a probe in the including file's
/// directory. Returns true when a computed include was seen.
pub fn include_closure(
    roots: &[PathBuf],
    quote_dirs: &[PathBuf],
    inc_dirs: &[PathBuf],
    repo: &Path,
    out: &mut TaskSpec,
) -> bool {
    let mut seen: BTreeSet<PathBuf> = roots.iter().cloned().collect();
    let mut todo: Vec<PathBuf> = roots.to_vec();
    let mut computed = false;
    while let Some(f) = todo.pop() {
        if seen.len() > MAX_HEADERS {
            out.uncertain(UncertaintyKind::PatternTooBroad, "too many #include files");
            return true;
        }
        let Ok(bytes) = fsx::read(&f) else { continue };
        let src = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_SOURCE)]).into_owned();
        let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
        for inc in includes(&src) {
            let (name, quoted) = match inc {
                Include::Quoted(n) => (n, true),
                Include::Angle(n) => (n, false),
                Include::Computed(d) => {
                    if !computed {
                        out.uncertain(
                            UncertaintyKind::ComputedSpecifier,
                            format!("computed #include {d}"),
                        );
                    }
                    computed = true;
                    continue;
                }
            };
            let mut search: Vec<&Path> = Vec::new();
            if quoted {
                search.push(&dir);
                search.extend(quote_dirs.iter().map(PathBuf::as_path));
            }
            search.extend(inc_dirs.iter().map(PathBuf::as_path));
            let found = search
                .iter()
                .map(|d| fsx::join(d, &name))
                .find(|p| p.starts_with(repo) && fsx::is_file(p));
            match found {
                Some(p) => {
                    if seen.insert(p.clone()) {
                        out.input(p.clone());
                        todo.push(p);
                    }
                }
                None if quoted => {
                    // Not in the repository (or not yet generated): creating it would change
                    // the build.
                    let p = fsx::join(&dir, &name);
                    if p.starts_with(repo) {
                        out.input(p);
                    }
                }
                None => {}
            }
        }
    }
    computed
}

enum Include {
    Quoted(String),
    Angle(String),
    Computed(String),
}

/// `#include`, `#import` and `#include_next` directives (comments are not
/// stripped: an include in a comment is an over-approximation).
fn includes(src: &str) -> Vec<Include> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = match ["include_next", "include", "import"]
            .iter()
            .find_map(|k| rest.strip_prefix(k))
        {
            Some(r) => r.trim_start(),
            None => continue,
        };
        if let Some(r) = rest.strip_prefix('"') {
            if let Some(e) = r.find('"') {
                out.push(Include::Quoted(r[..e].to_string()));
            }
        } else if let Some(r) = rest.strip_prefix('<') {
            if let Some(e) = r.find('>') {
                out.push(Include::Angle(r[..e].to_string()));
            }
        } else if !rest.is_empty() && !rest.starts_with("//") && !rest.starts_with("/*") {
            out.push(Include::Computed(rest.chars().take(40).collect()));
        }
    }
    out
}

/// `ar` (archives) and `ranlib`.
pub struct Ar;

impl Recognizer for Ar {
    fn id(&self) -> &'static str {
        "ar"
    }

    fn programs(&self) -> &'static [&'static str] {
        &["ar", "ranlib", "llvm-ar", "gcc-ar"]
    }

    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        if cmd.program_name() == Some("ranlib") {
            return Recognition::Ignored;
        }
        let words: Vec<&str> = cmd.args().filter(|w| !w.starts_with("--")).collect();
        let Some(ops) = words.first() else {
            return Recognition::Ignored;
        };
        let ops = ops.trim_start_matches('-');
        if !ops.contains(['r', 'q']) {
            return Recognition::Ignored;
        }
        let mut rest = words[1..].iter();
        if let Some(a) = rest.next() {
            out.output(cmd.path(a));
        }
        for m in rest {
            out.input(cmd.path(m));
        }
        out.tool("system", "ar");
        Recognition::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilers() {
        for n in [
            "cc",
            "gcc",
            "gcc-13",
            "arm-none-eabi-gcc",
            "clang++",
            "x86_64-linux-gnu-g++",
        ] {
            assert!(is_compiler(n), "{n}");
        }
        for n in ["gcc-ar", "ccache", "occ", "c"] {
            assert!(!is_compiler(n), "{n}");
        }
    }

    #[test]
    fn include_lines() {
        let v = includes(
            "#include \"a.h\"\n  # include <b/c.h>\n#import \"d.h\"\n#include FOO_H\n#define X\n",
        );
        assert!(matches!(&v[0], Include::Quoted(n) if n == "a.h"));
        assert!(matches!(&v[1], Include::Angle(n) if n == "b/c.h"));
        assert!(matches!(&v[2], Include::Quoted(n) if n == "d.h"));
        assert!(matches!(&v[3], Include::Computed(_)));
        assert_eq!(v.len(), 4);
    }
}
