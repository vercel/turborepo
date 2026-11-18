//! The strings of a Go file that can be pieces of a path, for the bound on
//! reads with a computed path (`turborepo_trace_graph::names`).
//!
//! `os.ReadFile(path)` with `path` a parameter, a struct field or the result of
//! a helper is a read the extractor cannot evaluate. The path is still text
//! that the running code assembled, so a change query bounds the read by the
//! path-like strings of the code loaded with it. This module is the Go half of
//! the collection: one pass over the tokens, a pure function of the file's
//! text. Which strings are names is decided by the shared predicate
//! (`turborepo_trace_lang_js::names::of_strings`), so that a Go string and a JS
//! string of the same shape name the same files.
//!
//! What is collected:
//!
//! | Fact | Source |
//! |---|---|
//! | [`Literal`] | every interpreted and raw string literal outside import declarations. In a call of a formatting function (`fmt.Sprintf`, any callee ending in `f`) the verbs are holes ([`HOLE`]): `"configs/%s.yaml"` names `configs` |
//! | [`Literal::path_arg`] | the literal is a direct argument of a function that takes pieces of a path (`filepath.Join`, `os.ReadFile`, `os.Stat`, ...; by name, on any receiver): there a bare word is a name (`filepath.Join(dir, "fixtures", name)`). Consecutive literal arguments are one literal, joined with `/`: `filepath.Join(dir, "src", "routes.ts")` is `src/routes.ts`, which names far less than `src` |
//! | [`Strings::location`] | a use of the file `runtime.Caller` reports, other than in a literal path to a file: `filepath.Join(filepath.Dir(file), "..", "schemas", "job.json")` is a reference to exactly that file (the extractor emits it), and its `..` and `Dir` are no parent steps of anything else. A path that only goes up, names a directory, or has a part that is not a literal, uses the location: [`names`] then follows where it goes ([`crate::anchors`]), and only a file with a use that cannot be followed names everything below its location |
//! | [`Strings::getwd`] | `os.Getwd` |
//! | [`Strings::parents`] | `filepath.Dir` / `path.Dir` and `filepath.Split` / `path.Split` calls, through an alias or a dot import too: steps toward the root (`..` segments are counted from the strings) |
//! | [`Strings::dir_in_loop`] | such a call can run more than once: anywhere but at package level or directly in a test function outside loops (`steps_repeat`) |
//!
//! # Strings that climb
//!
//! A relative path in Go is relative to the working directory of the process,
//! and `go test` runs a package's tests in the package directory. So a string
//! that climbs `k` directories with leading parent steps
//! (`os.ReadFile("../../../package.json")`, `filepath.Join("..", "..",
//! "shared")`) names its rest at the directories `1..=k` above the working
//! directory, or below it when it is joined onto a deeper base: not its shape
//! anywhere in the repository. The name is marked with how far it can climb
//! ([`Name::climbs_from_cwd`](turborepo_trace_core::names::Name::climbs_from_cwd)):
//! its own steps plus every other parent step of the file (its other `..`
//! segments and its `filepath.Dir` calls), which can raise the base it is
//! joined onto. The query adds the parent steps of the other Go files of the
//! package (the files of the directory), whose strings and calls can be that
//! base too (`const Fixtures = "../../../fixtures"` in one file,
//! `filepath.Join(Fixtures, "../shared/a.json")` in another). It names the
//! rest at those directories, in whatever package they are, the rest
//! continued within its last segment there too (`"../../shared/golden" +
//! ".json"`, `"../../shared/app." + env + ".yaml"`), and the shape, continued
//! too, anywhere below the working directory, and inside the package like any
//! other string (`turborepo_trace_graph::names`, "Strings that climb from the
//! working directory"). This assumes:
//!
//! > A Go string that climbs with parent steps is taken from the working
//! > directory the test starts in, or from a directory below it: it is not
//! > joined onto a path that another package builds above the working directory
//! > or elsewhere (a directory that another package finds from its own
//! > location, the value of an environment variable), and no other package
//! > changes the working directory (`os.Chdir` in the `init` of an imported
//! > package).
//!
//! The file's own location names its own places ([`crate::anchors`]), and the
//! consecutive literal arguments of one `filepath.Join` call are one string.
//! What the analysis can see of a further climb keeps the plain climbing mark,
//! whose shape names anywhere:
//!
//! - a string whose rest is not its shape: a named segment left before a later
//!   parent step (`"../a/b/../c"`) or a hole before the name (`"../%s/data"`);
//! - every climbing string of a file whose parent steps can run more than once
//!   (`filepath.Dir` in a loop or in its clauses, in a function literal, in a
//!   helper function, in a file with `goto`; [`Strings::dir_in_loop`]);
//! - every climbing string of a package (the files of one directory) in which a
//!   file calls `Chdir` (`os.Chdir`, `t.Chdir`, a `TestMain` that moves), holds
//!   a string that only climbs (`"../.."`, `const Root = "../.."`,
//!   `filepath.Join("..", "..")`), has parent steps that can run more than once
//!   (an upward search for `go.mod`), or follows its own location above its
//!   directory (`filepath.Join(filepath.Dir(file), "..", "..")` from
//!   `runtime.Caller`): such a base can be joined to the strings of the other
//!   files ([`Names::moves_base`](turborepo_trace_core::names::Names::moves_base)).
//!   The parent steps of a file without names count for the others too: its
//!   fact is kept for them.
//!
//! A call that changes the working directory (not into a temporary directory,
//! [`chdir_calls`]) moves every relative path of the directory's files, the
//! literal reads of its test files too, which are otherwise taken from the
//! package directory. The extractor records it as a read with an unknown path
//! ([`crate::extract::CHDIR_DETAIL`]), which every test of the directory
//! reaches: `--strict` opens it, and by default it is bounded by names. The
//! file is marked
//! ([`Names::chdir`](turborepo_trace_core::names::Names::chdir)),
//! and every name of the directory's Go files then names its shape anywhere,
//! not only inside the package. What a string of another package names after
//! the change is still held to its package.
//!
//! What is left out, because the string never becomes a path:
//!
//! - the arguments after the first of a `strings` / `bytes` function that only
//!   tests or cuts its first argument (`strings.TrimSuffix(dir, "/src")`,
//!   `strings.HasPrefix(p, "testdata/")`): they are in no result;
//! - the message of a failure, a log line or an error: direct arguments of
//!   `t.Fatalf`, `t.Errorf`, `t.Logf`, `t.Skipf` and their plain forms,
//!   `fmt.Errorf`, `errors.New`, `fmt.Print*`, `log.Print*`, `panic`;
//! - a path joined below a temporary directory in the same call
//!   (`filepath.Join(t.TempDir(), "config.json")`, `filepath.Join(dir,
//!   "a.txt")` with `dir := t.TempDir()`): what a test writes there and reads
//!   back is no repository file ([`crate::temp`]).
//!
//! Not seen: strings built from fragments none of which is a name, identifiers
//! used as path segments, and text that is not in the scanned files (the
//! environment, a module that is not in the repository).

/// The hole marker in [`Literal::value`]: a part that is not literal text (a
/// formatting verb).
pub use turborepo_trace_core::names::HOLE;

use crate::{
    anchors::Followed,
    lex::{Kind, Tok, string_value, tokenize},
    temp::TempRoots,
};

/// Longest literal kept.
const MAX_STRING: usize = 8192;

/// Largest value of [`Strings::parents`].
const MAX_PARENTS: u32 = 64;

/// One string literal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Literal {
    /// The decoded value, formatting verbs replaced by [`HOLE`].
    pub value: String,
    /// A direct argument of a path function.
    pub path_arg: bool,
    /// 1-based line.
    pub line: u32,
}

/// The path-relevant facts of one Go file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strings {
    /// String literals in source order.
    pub literals: Vec<Literal>,
    /// Line of the first `runtime.Caller`.
    pub location: Option<u32>,
    /// Line of the first `os.Getwd`.
    pub getwd: Option<u32>,
    /// Number of `filepath.Dir` / `path.Dir` calls, at most 64.
    pub parents: u32,
    /// The file calls a function called `Chdir` (`os.Chdir`, `t.Chdir`): its
    /// relative paths may be taken from another working directory.
    pub chdir: bool,
    /// A counted parent step (`filepath.Dir`, `filepath.Split`) can run more
    /// than once, so [`Strings::parents`] does not bound how far it climbs
    /// (`steps_repeat`): it is in a loop, in a function literal, in a
    /// function that other code calls, or the file has a `goto`.
    pub dir_in_loop: bool,
}

/// Functions whose string arguments are pieces of a path (matched by name on
/// any receiver).
fn path_fn(name: &[u8]) -> bool {
    matches!(
        name,
        b"Join"
            | b"Abs"
            | b"Rel"
            | b"Clean"
            | b"Dir"
            | b"Base"
            | b"Glob"
            | b"Match"
            | b"Walk"
            | b"WalkDir"
            | b"EvalSymlinks"
            | b"ReadFile"
            | b"WriteFile"
            | b"Open"
            | b"OpenFile"
            | b"OpenRoot"
            | b"OpenInRoot"
            | b"Create"
            | b"ReadDir"
            | b"DirFS"
            | b"Sub"
            | b"Stat"
            | b"Lstat"
            | b"Mkdir"
            | b"MkdirAll"
            | b"Remove"
            | b"RemoveAll"
            | b"Rename"
            | b"Symlink"
            | b"Link"
            | b"Readlink"
            | b"Chmod"
            | b"Chdir"
            | b"Truncate"
            | b"ParseFiles"
            | b"ParseGlob"
            | b"ParseFS"
            | b"ServeFile"
            | b"LoadFile"
    )
}

/// Functions whose string arguments are messages: the text goes to the test
/// log, the standard streams or an error value, never into a path.
pub(crate) fn message_fn(name: &[u8], receiver: Option<&[u8]>) -> bool {
    match name {
        b"Fatalf" | b"Fatal" | b"Errorf" | b"Logf" | b"Skipf" | b"Panicf" | b"Printf"
        | b"Println" | b"Print" | b"panic" => true,
        // `t.Error`, `t.Log`, `t.Skip`: only on a receiver (a function called `Error` on its
        // own is something else), and `errors.New`.
        b"Error" | b"Log" | b"Skip" => receiver.is_some(),
        b"New" => receiver == Some(b"errors"),
        _ => false,
    }
}

/// Functions of `strings` and `bytes` whose arguments after the first only test
/// or cut the first: the result is a truth value, a position, or a part of the
/// first argument, never text that contains the other arguments.
/// `strings.TrimSuffix(dir, "/src")` does not make `src` a piece of a path.
/// (`Replace` and `Join` are not in the list: their arguments end
/// up in the result.)
fn probe_fn(name: &[u8], receiver: Option<&[u8]>) -> bool {
    matches!(receiver, Some(b"strings" | b"bytes"))
        && matches!(
            name,
            b"HasPrefix"
                | b"HasSuffix"
                | b"Contains"
                | b"ContainsAny"
                | b"Index"
                | b"IndexAny"
                | b"LastIndex"
                | b"LastIndexAny"
                | b"Count"
                | b"Compare"
                | b"EqualFold"
                | b"TrimPrefix"
                | b"TrimSuffix"
                | b"Trim"
                | b"TrimLeft"
                | b"TrimRight"
                | b"Split"
                | b"SplitN"
                | b"SplitAfter"
                | b"SplitAfterN"
                | b"Cut"
                | b"CutPrefix"
                | b"CutSuffix"
        )
}

/// Whether `s` is a literal path to a file: made of `.`, `..` and plain
/// segments, the last one a file name with an extension
/// (`specs/order.json`, `../../data/a.sql`).
///
/// A literal path from the file's own location is a reference to exactly that
/// file when it looks like this ([`crate::extract`] emits it, and the names
/// scan does not count it as a use of the location). A path to a directory is
/// not: what is read below it is unknown.
pub fn literal_file_path(s: &str) -> bool {
    let plain = |seg: &str| {
        seg.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'@' | b'+' | b'~' | b'-')
        }) && seg.bytes().any(|c| c.is_ascii_alphanumeric())
    };
    let mut last = "";
    for seg in s.split('/') {
        match seg {
            "" => {}
            // A dot segment after a name: not a plain path down to a file.
            "." | ".." => last = "",
            seg if plain(seg) => last = seg,
            _ => return false,
        }
    }
    last.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && !ext.is_empty()
            && ext.len() <= 10
            && ext.bytes().all(|c| c.is_ascii_alphanumeric())
            && ext.bytes().any(|c| c.is_ascii_alphabetic())
    })
}

/// Replaces the verbs of a format string by [`HOLE`] (`%%` is a literal `%`).
fn verbs_to_holes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if it.peek() == Some(&'%') {
            it.next();
            out.push('%');
            continue;
        }
        // Flags, width, precision, argument index, then the verb.
        while let Some(&n) = it.peek() {
            if matches!(n, '+' | '-' | '#' | ' ' | '0'..='9' | '.' | '*' | '[' | ']') {
                it.next();
            } else {
                break;
            }
        }
        if it.peek().is_some_and(|n| n.is_ascii_alphabetic()) {
            it.next();
        }
        out.push(HOLE);
    }
    out
}

/// An open bracket.
#[derive(Copy, Clone, Default)]
struct Group {
    /// `(` of a path function.
    path: bool,
    /// `(` of a message function.
    message: bool,
    /// `(` of a function whose name ends in `f` (a format string follows).
    format: bool,
    /// `(` of a path function whose first argument is rooted in a temporary
    /// directory.
    temp: bool,
    /// `(` of a `strings` / `bytes` function that tests or cuts its first
    /// argument with the others ([`probe_fn`]).
    probe: bool,
    /// Commas seen directly in the group: the index of the current argument.
    commas: u32,
    /// The last literal collected directly in this group: its token (an index
    /// into the significant tokens) and its place in [`Strings::literals`].
    last: Option<(usize, usize)>,
}

/// The variables that hold the caller file: the second target of every
/// `pc, file, line, ok := runtime.Caller(…)`, with the token (an index into the
/// significant tokens `sig`) where it is declared and the token of that
/// `Caller`.
fn caller_files<'s>(src: &'s [u8], toks: &[Tok], sig: &[usize]) -> Vec<(&'s [u8], usize, usize)> {
    let tok = |k: usize| toks[sig[k]];
    let mut out = Vec::new();
    for k in 3..sig.len() {
        if !(tok(k).is_ident(src, b"Caller")
            && tok(k - 1).is_punct(src, b'.')
            && tok(k - 2).is_ident(src, b"runtime")
            && tok(k - 3).is_punct(src, b'='))
        {
            continue;
        }
        // Targets, right to left, from before `=` or `:=`.
        let mut j = k - 3;
        if j > 0 && tok(j - 1).is_punct(src, b':') {
            j -= 1;
        }
        let mut targets: Vec<usize> = Vec::new();
        while j > 0 && tok(j - 1).kind == Kind::Ident {
            targets.push(j - 1);
            j -= 1;
            if j > 0 && tok(j - 1).is_punct(src, b',') {
                j -= 1;
            } else {
                break;
            }
        }
        targets.reverse();
        if let Some(&file) = targets.get(1)
            && tok(file).text(src) != b"_"
        {
            out.push((tok(file).text(src), file, k));
        }
    }
    out
}

/// Whether the call whose `(` is the significant token `open` is a literal path
/// from the file's own location: `filepath.Join(filepath.Dir(<caller file>),
/// "lit", "lit", …)` with only string literals after the first argument, which
/// together are a path to a file ([`literal_file_path`]). Returns the closing
/// parenthesis.
///
/// Such a call is a reference to exactly that path (the extractor evaluates
/// it), not a use of the location as a base for something unknown: its `..` and
/// its `Dir` move nothing else.
fn location_literal(
    src: &[u8],
    toks: &[Tok],
    sig: &[usize],
    open: usize,
    files: &[(&[u8], usize, usize)],
) -> Option<usize> {
    let tok = |k: usize| sig.get(k).map(|&i| toks[i]);
    let punct = |k: usize, c: u8| tok(k).is_some_and(|t| t.is_punct(src, c));
    let word = |k: usize, w: &[u8]| tok(k).is_some_and(|t| t.is_ident(src, w));
    // `filepath . Dir ( file )`
    if !((word(open + 1, b"filepath") || word(open + 1, b"path"))
        && punct(open + 2, b'.')
        && word(open + 3, b"Dir")
        && punct(open + 4, b'(')
        && tok(open + 5)
            .is_some_and(|t| t.kind == Kind::Ident && files.iter().any(|f| f.0 == t.text(src)))
        && punct(open + 6, b')'))
    {
        return None;
    }
    let mut k = open + 7;
    let mut path = String::new();
    while punct(k, b',') {
        let t = tok(k + 1)?;
        if !t.is_string() {
            return None;
        }
        path.push('/');
        path.push_str(&string_value(src, &t)?);
        k += 2;
    }
    (punct(k, b')') && literal_file_path(&path)).then_some(k)
}

/// The calls of the Go file `src` that change the working directory: a call of
/// a function called `Chdir` (`os.Chdir(dir)`, `t.Chdir(dir)`, on any
/// receiver) whose first argument is not below a temporary directory
/// ([`crate::temp`]: what a test reads there after it is no repository file).
/// The span of each `Chdir`. A declaration of a function or method called
/// `Chdir` is not a call.
pub fn chdir_calls(src: &[u8]) -> Vec<turborepo_trace_core::Span> {
    if memchr::memmem::find(src, b"Chdir").is_none() {
        return Vec::new();
    }
    let (toks, _) = tokenize(src);
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| !toks[i].is_comment()).collect();
    let roots = TempRoots::possible(src).then(|| TempRoots::of(src, &toks));
    let tok = |k: usize| toks[sig[k]];
    let mut out = Vec::new();
    for k in 0..sig.len() {
        if !(tok(k).is_ident(src, b"Chdir")
            && sig.get(k + 1).is_some_and(|&i| toks[i].is_punct(src, b'(')))
        {
            continue;
        }
        // `func Chdir(` and `func (r T) Chdir(` declare it.
        let declared = k > 0
            && (tok(k - 1).is_ident(src, b"func")
                || (tok(k - 1).is_punct(src, b')') && {
                    let mut depth = 0i32;
                    let mut j = k - 1;
                    loop {
                        let t = tok(j);
                        if t.is_punct(src, b')') {
                            depth += 1;
                        } else if t.is_punct(src, b'(') {
                            depth -= 1;
                            if depth == 0 {
                                break j > 0 && tok(j - 1).is_ident(src, b"func");
                            }
                        }
                        if j == 0 {
                            break false;
                        }
                        j -= 1;
                    }
                }));
        if declared {
            continue;
        }
        if roots
            .as_ref()
            .is_some_and(|r| r.call_below_temp(src, &toks, sig[k + 1]))
        {
            continue;
        }
        out.push(tok(k).span);
    }
    out
}

/// The path-relevant facts of the Go file `src`.
pub fn strings(src: &[u8]) -> Strings {
    scan(src, None)
}

/// The facts of `src`. With `followed` (the file's location was followed,
/// [`crate::anchors`]) the parent steps that are part of its entries are not
/// counted again: the `filepath.Dir` calls it evaluated, and the `..` of the
/// literals it joined, of which only what follows the last parent step is kept
/// as a string.
fn scan(src: &[u8], followed: Option<&Followed>) -> Strings {
    let (toks, _) = tokenize(src);
    let mut out = Strings::default();
    // The literals that are part of a followed path.
    let mut joined_at: Vec<usize> = Vec::new();
    let part = |offsets: &[u32], at: usize| offsets.binary_search(&(at as u32)).is_ok();
    let roots = TempRoots::possible(src).then(|| TempRoots::of(src, &toks));
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| !toks[i].is_comment()).collect();
    let files = if memchr::memmem::find(src, b"runtime.Caller").is_some() {
        caller_files(src, &toks, &sig)
    } else {
        Vec::new()
    };
    // The closing parenthesis of the literal location path the scan is inside of.
    let mut literal_until: Option<usize> = None;
    let mut groups: Vec<Group> = Vec::new();
    // The names `path/filepath` and `path` are imported as, and whether one of them
    // is imported into the file's scope (`import . "path/filepath"`).
    let (pkgs, dot_pkg) = path_packages(src, &toks, &sig);
    // The counted parent steps: the significant token of each `Dir` / `Split`.
    let mut steps: Vec<usize> = Vec::new();
    let mut line = 1u32;
    let mut line_at = 0usize;
    let mut k = 0;
    while k < sig.len() {
        let t = toks[sig[k]];
        let at = t.span.start as usize;
        if at > line_at {
            line += memchr::memchr_iter(b'\n', &src[line_at..at]).count() as u32;
            line_at = at;
        }
        let here = k;
        let prev = |n: usize| here.checked_sub(n).map(|p| toks[sig[p]]);
        let next = |n: usize| sig.get(here + n).map(|&i| toks[i]);
        // Import declarations are references already.
        if t.is_ident(src, b"import") && !prev(1).is_some_and(|p| p.is_punct(src, b'.')) {
            k += 1;
            if next(1).is_some_and(|n| n.is_punct(src, b'(')) {
                while k < sig.len() && !toks[sig[k]].is_punct(src, b')') {
                    k += 1;
                }
            } else {
                while k < sig.len() && !toks[sig[k]].is_string() {
                    k += 1;
                }
            }
            k += 1;
            continue;
        }
        let in_literal = literal_until.is_some_and(|end| here <= end);
        match t.kind {
            Kind::Punct => match src[at] {
                b'(' => {
                    let callee = prev(1).filter(|p| p.kind == Kind::Ident);
                    let receiver = prev(2)
                        .filter(|p| p.is_punct(src, b'.'))
                        .and_then(|_| prev(3))
                        .filter(|p| p.kind == Kind::Ident)
                        .map(|p| p.text(src));
                    let name = callee.map(|c| c.text(src)).unwrap_or(b"");
                    let path = callee.is_some() && path_fn(name);
                    // The first argument is a temp root name or a `TempDir()` call.
                    let temp = path
                        && roots
                            .as_ref()
                            .is_some_and(|r| r.call_below_temp(src, &toks, sig[k]));
                    if name == b"Join" && !in_literal && !files.is_empty() {
                        literal_until = location_literal(src, &toks, &sig, here, &files);
                    }
                    groups.push(Group {
                        path,
                        message: callee.is_some() && message_fn(name, receiver),
                        format: name.last() == Some(&b'f') && name.len() > 1,
                        temp,
                        probe: callee.is_some() && probe_fn(name, receiver),
                        commas: 0,
                        last: None,
                    });
                }
                b'{' | b'[' => groups.push(Group::default()),
                b')' | b']' | b'}' => {
                    let g = groups.pop();
                    // The literal location path is complete: it names what is below its
                    // parent steps, which are not parent steps of anything else.
                    if literal_until == Some(here)
                        && let Some((_, slot)) = g.and_then(|g| g.last)
                    {
                        let l = &mut out.literals[slot];
                        l.value = l
                            .value
                            .split('/')
                            .skip_while(|s| matches!(*s, "" | "." | ".."))
                            .collect::<Vec<_>>()
                            .join("/");
                    }
                }
                b',' => {
                    if let Some(g) = groups.last_mut() {
                        g.commas += 1;
                    }
                }
                _ => {}
            },
            Kind::Ident => {
                let after = |w: &[u8]| {
                    prev(1).is_some_and(|p| p.is_punct(src, b'.'))
                        && prev(2).is_some_and(|p| p.is_ident(src, w))
                };
                let name = t.text(src);
                match name {
                    // A call whose file is not bound to a variable is the use itself.
                    b"Caller" if after(b"runtime") && !files.iter().any(|f| f.2 == here) => {
                        out.location.get_or_insert(line);
                    }
                    b"Getwd" if after(b"os") => {
                        out.getwd.get_or_insert(line);
                    }
                    b"Chdir" if next(1).is_some_and(|n| n.is_punct(src, b'(')) => {
                        out.chdir = true;
                    }
                    // `filepath.Split` gives the directory too.
                    b"Dir" | b"Split"
                        if !in_literal
                            && !followed.is_some_and(|f| part(&f.dirs, at))
                            && (pkgs.iter().any(|&p| after(p))
                                || (dot_pkg
                                    && !prev(1).is_some_and(|p| p.is_punct(src, b'.'))))
                            && next(1).is_some_and(|n| n.is_punct(src, b'(')) =>
                    {
                        out.parents = (out.parents + 1).min(MAX_PARENTS);
                        steps.push(here);
                    }
                    // A use of the caller file outside a literal path from it (and not its
                    // declaration, a field or a method of that name).
                    _ if !in_literal
                        && files.iter().any(|f| f.0 == name)
                        && !files.iter().any(|f| f.1 == here)
                        && !prev(1).is_some_and(|p| p.is_punct(src, b'.')) =>
                    {
                        out.location.get_or_insert(line);
                    }
                    _ => {}
                }
            }
            Kind::Str | Kind::RawStr => {
                let g = groups.last().copied().unwrap_or_default();
                if !g.message
                    && !g.temp
                    && !(g.probe && g.commas >= 1)
                    && t.span.range().len() <= MAX_STRING
                    && let Some(v) = string_value(src, &t)
                    && !v.is_empty()
                {
                    let value = if g.format {
                        verbs_to_holes(&v)
                    } else {
                        v.into_owned()
                    };
                    // Consecutive literal arguments of a path function are consecutive
                    // segments of one path: `Join(dir, "src", "routes.ts")` names
                    // `src/routes.ts`, not everything below a directory called `src`.
                    let joined = g.path
                        && here >= 2
                        && prev(1).is_some_and(|p| p.is_punct(src, b','))
                        && g.last.is_some_and(|(tok, _)| tok == here - 2);
                    let slot = match g.last {
                        Some((_, i)) if joined => {
                            let l = &mut out.literals[i];
                            l.value.push('/');
                            l.value.push_str(&value);
                            i
                        }
                        _ => {
                            if followed.is_some_and(|f| part(&f.literals, at)) {
                                joined_at.push(out.literals.len());
                            }
                            out.literals.push(Literal {
                                value,
                                path_arg: g.path,
                                line,
                            });
                            out.literals.len() - 1
                        }
                    };
                    if let Some(top) = groups.last_mut() {
                        top.last = Some((here, slot));
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    for i in joined_at {
        let l = &mut out.literals[i];
        let segs: Vec<&str> = l.value.split('/').collect();
        let from = segs
            .iter()
            .rposition(|s| matches!(*s, "." | ".."))
            .map_or(0, |i| i + 1);
        l.value = segs[from..].join("/");
    }
    out.literals.retain(|l| !l.value.is_empty());
    out.dir_in_loop = steps_repeat(src, &toks, &sig, &steps);
    out
}

/// The names the file imports `path/filepath` and `path` as (`filepath`,
/// `path`, or an alias: `fp "path/filepath"`), and whether one of them is
/// imported into the file's scope (`import . "path/filepath"`).
fn path_packages<'s>(src: &'s [u8], toks: &[Tok], sig: &[usize]) -> (Vec<&'s [u8]>, bool) {
    let mut names: Vec<&'s [u8]> = vec![b"filepath", b"path"];
    let mut dot = false;
    let tok = |k: usize| toks[sig[k]];
    let mut k = 0;
    while k < sig.len() {
        if !(tok(k).is_ident(src, b"import") && (k == 0 || !tok(k - 1).is_punct(src, b'.'))) {
            k += 1;
            continue;
        }
        let grouped = sig.get(k + 1).is_some_and(|&i| toks[i].is_punct(src, b'('));
        k += 1 + usize::from(grouped);
        while k < sig.len() {
            let t = tok(k);
            if t.is_punct(src, b')') {
                break;
            }
            if t.is_string() {
                let value = string_value(src, &t);
                if matches!(value.as_deref(), Some("path/filepath" | "path")) && k > 0 {
                    let b = tok(k - 1);
                    if b.kind == Kind::Ident && !b.is_ident(src, b"import") {
                        names.push(b.text(src));
                    } else if b.is_punct(src, b'.') {
                        dot = true;
                    }
                }
                if !grouped {
                    break;
                }
            }
            k += 1;
        }
        k += 1;
    }
    (names, dot)
}

/// Whether one of the counted parent steps of a file (`steps`: the significant
/// tokens of the `Dir` / `Split` calls) can run more than once, so that their
/// number does not bound how far a path climbs. A step is taken to run once
/// only where that is plain from the file: at package level, or directly in
/// the body of a function that the test framework calls once and no code of
/// the file calls (`TestX`, `BenchmarkX`, `FuzzX`, `ExampleX`, `TestMain`,
/// `init`, `main`), outside every loop (its clauses too) and every function
/// literal, in a file without `goto`. Any other step can repeat: in a loop, in
/// a function literal (a closure called in a loop), in a helper (called from
/// a loop, recursively, more than once, or from another file).
fn steps_repeat(src: &[u8], toks: &[Tok], sig: &[usize], steps: &[usize]) -> bool {
    if steps.is_empty() {
        return false;
    }
    let tok = |k: usize| toks[sig[k]];
    if (0..sig.len()).any(|k| tok(k).is_ident(src, b"goto")) {
        return true;
    }
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum Frame {
        Plain,
        /// The braces of a `struct` or `interface` type.
        Type,
        Loop,
        Literal,
        Function,
    }
    let entry = |name: &[u8]| {
        matches!(name, b"init" | b"main" | b"TestMain")
            || [&b"Test"[..], b"Benchmark", b"Fuzz", b"Example"]
                .iter()
                .any(|p| name.starts_with(p))
    };
    let newline_before = |k: usize| {
        k > 0 && {
            let (a, b) = (tok(k - 1).span.end as usize, tok(k).span.start as usize);
            a < b && memchr::memchr(b'\n', &src[a..b]).is_some()
        }
    };
    let mut frames: Vec<Frame> = Vec::new();
    // Depths of the `for` statements whose body is still to come, and of the
    // function literals (or types) whose body is still to come.
    let mut for_at: Vec<usize> = Vec::new();
    let mut literal_at: Vec<usize> = Vec::new();
    // A top-level function whose body is still to come, and the one whose body
    // the scan is in.
    let mut declared: Option<&[u8]> = None;
    let mut current: Option<&[u8]> = None;
    // The last brace closed was a type's.
    let mut closed_type = false;
    let mut next_step = 0usize;
    for k in 0..sig.len() {
        let t = tok(k);
        let depth = frames.len();
        // A signature without a body ends its line.
        if newline_before(k) {
            if literal_at.last() == Some(&depth) {
                literal_at.pop();
            }
            if depth == 0 {
                declared = None;
            }
        }
        if next_step < steps.len() && steps[next_step] == k {
            next_step += 1;
            let repeats = !for_at.is_empty()
                || !literal_at.is_empty()
                || frames
                    .iter()
                    .any(|f| matches!(f, Frame::Loop | Frame::Literal))
                || current.is_some_and(|name| {
                    !entry(name)
                        || (0..sig.len())
                            .filter(|&j| tok(j).is_ident(src, name))
                            .count()
                            > 1
                });
            if repeats {
                return true;
            }
        }
        let prev = |n: usize| k.checked_sub(n).map(tok);
        match t.kind {
            Kind::Ident if !prev(1).is_some_and(|p| p.is_punct(src, b'.')) => match t.text(src) {
                b"for" => for_at.push(depth),
                b"func" if depth == 0 => {
                    // `func Name(`, `func (r T) Name(`, `func Name[T any](`.
                    let mut j = k + 1;
                    if j < sig.len() && tok(j).is_punct(src, b'(') {
                        let mut open = 0i32;
                        while j < sig.len() {
                            if tok(j).is_punct(src, b'(') {
                                open += 1;
                            } else if tok(j).is_punct(src, b')') {
                                open -= 1;
                                if open == 0 {
                                    break;
                                }
                            }
                            j += 1;
                        }
                        j += 1;
                    }
                    declared =
                        (j < sig.len() && tok(j).kind == Kind::Ident).then(|| tok(j).text(src));
                    if declared.is_none() {
                        declared = Some(b"");
                    }
                }
                b"func" => literal_at.push(depth),
                _ => {}
            },
            Kind::Punct => match src[t.span.start as usize] {
                b'{' => {
                    let type_brace = prev(1).is_some_and(|p| {
                        p.is_ident(src, b"struct") || p.is_ident(src, b"interface")
                    });
                    // A composite literal in a loop's clauses: `[]T{`, `[]*T{`, `[]pkg.T{`,
                    // `map[K]V{`, `[]struct{ … }{`. A type name alone before the brace
                    // would need parentheses there, so another brace is the body.
                    let composite = (prev(1).is_some_and(|p| p.is_punct(src, b'}')) && closed_type)
                        || (prev(1).is_some_and(|p| p.kind == Kind::Ident) && {
                            let mut j = 2;
                            if prev(2).is_some_and(|p| p.is_punct(src, b'.'))
                                && prev(3).is_some_and(|p| p.kind == Kind::Ident)
                            {
                                j = 4;
                            }
                            while prev(j).is_some_and(|p| p.is_punct(src, b'*')) {
                                j += 1;
                            }
                            prev(j).is_some_and(|p| p.is_punct(src, b']'))
                        });
                    let frame = if type_brace {
                        Frame::Type
                    } else if declared.is_some() && depth == 0 {
                        current = declared.take().filter(|n| !n.is_empty());
                        if current.is_none() {
                            current = Some(b"");
                        }
                        Frame::Function
                    } else if literal_at.last() == Some(&depth) {
                        literal_at.pop();
                        Frame::Literal
                    } else if for_at.last() == Some(&depth) && !composite {
                        for_at.pop();
                        Frame::Loop
                    } else {
                        Frame::Plain
                    };
                    frames.push(frame);
                }
                b'(' | b'[' => frames.push(Frame::Plain),
                b')' | b']' | b'}' => {
                    let f = frames.pop();
                    closed_type = f == Some(Frame::Type);
                    if f == Some(Frame::Function) {
                        current = None;
                    }
                    let depth = frames.len();
                    for_at.retain(|&d| d <= depth);
                    literal_at.retain(|&d| d <= depth);
                }
                _ => {}
            },
            _ => {}
        }
    }
    false
}

/// Adds the names of the literal `l` to `c`, as
/// [`Collector::string`](turborepo_trace_core::names::Collector::string)
/// does, with what a string that climbs says about where it goes.
///
/// A relative path in Go is relative to the working directory of the process
/// (`go test` runs a package's tests in the package directory). A string that
/// climbs (`"../../shared/a.json"`, consecutive literal arguments of
/// `filepath.Join` merged) names its rest `R` at the directories `1..=k` above
/// the working directory, and below it when it is joined onto a deeper base
/// (`filepath.Join("a/b", "../../../x")`): the name is marked with that `k`
/// ([`Name::climbs_from_cwd`](turborepo_trace_core::names::Name::climbs_from_cwd)).
/// `k` is the string's own climb plus every other parent step the file has
/// (`raise`: its other `..` segments and its `filepath.Dir` calls), which can
/// raise the base the string is joined onto.
///
/// A string whose rest is not its name keeps the plain climbing mark (its shape
/// names anywhere): one with a named segment left before a later parent step
/// (`"../a/b/../c"` is `c` by shape, `a/c` by place) or a hole before its name
/// (`"../%s/data"`). So does every climbing string of a file whose base can
/// climb further than its parent steps say (`plain`: it changes its working
/// directory, or calls `filepath.Dir` in a loop). A string that only climbs
/// (`"../.."`) can be the base of another one: it marks the file
/// ([`Names::moves_base`](turborepo_trace_core::names::Names::moves_base)), and
/// the query then keeps the plain mark for the whole package.
fn collect(c: &mut turborepo_trace_core::names::Collector, l: &Literal, raise: u32, plain: bool) {
    use turborepo_trace_core::names::{MAX_STRING, MAX_UPS, climb_of, separates, ups_of, words};
    if l.value.len() > MAX_STRING {
        return;
    }
    let whole = !l.value.chars().any(separates);
    for w in words(&l.value) {
        let own = ups_of(w);
        c.names.ups = (c.names.ups + own).min(MAX_UPS);
        let (up, down) = climb_of(w);
        if up > 0 && down.is_empty() {
            c.names.moves_base = true;
        }
        let Some(i) = c.word_at(w, l.path_arg && whole, l.line) else {
            continue;
        };
        if up == 0 {
            continue;
        }
        let shape = &c.names.names[i].shape;
        let n = shape.split('/').count();
        if !plain && down.len() >= n && shape.split('/').zip(&down).all(|(a, b)| a == *b) {
            c.mark_climbs_from_cwd(i, up.saturating_add(raise.saturating_sub(own)));
        } else {
            c.mark_climbs(i);
        }
    }
}

/// The parent steps of the strings of `literals` (`..` segments of the words
/// [`collect`] looks at).
fn dots(literals: &[Literal]) -> u32 {
    use turborepo_trace_core::names::{MAX_STRING, ups_of, words};
    literals
        .iter()
        .filter(|l| l.value.len() <= MAX_STRING)
        .flat_map(|l| words(&l.value))
        .map(ups_of)
        .fold(0u32, u32::saturating_add)
}

/// The names of the Go file `src` (`turborepo_trace_core::names`): its strings
/// by the predicate every language shares, and its anchors.
///
/// - `runtime.Caller` is the file's own location. It gives the file, not its
///   directory: the first parent step (`filepath.Dir(file)`, or the first `..`
///   joined to it) only removes the file name (the query side takes one step
///   off for a Go file);
/// - a `_test.go` file (`test_file`) always uses the working directory. `go
///   test` runs a package's tests in the package directory, and a relative path
///   needs no string to name what is there: `"."`, a name taken from a
///   directory listing, an empty prefix. So a test with a read that can't be
///   evaluated is taken to read anything below its package directory. Its own
///   package files and `testdata` are inputs of the test anyway; what this adds
///   are the other files and sub-directories of the package directory;
/// - any Go file that calls `os.Getwd` uses the working directory too;
/// - a string that climbs names its rest at the directories above the working
///   directory it climbs to (see "Strings that climb" in the module
///   documentation).
///
/// The location is followed through the file ([`crate::anchors`]): the names
/// then say where it goes instead of everything below it.
pub fn names(src: &[u8], test_file: bool) -> turborepo_trace_core::names::Names {
    names_with(src, test_file, true)
}

/// As [`names`]; `follow`: whether the file's location is followed
/// ([`crate::anchors`]). Without, or when a use of it cannot be followed, the
/// file names everything below its location, raised by all its parent steps.
pub fn names_with(src: &[u8], test_file: bool, follow: bool) -> turborepo_trace_core::names::Names {
    use turborepo_trace_core::names::{Collector, MAX_UPS};
    let mut s = strings(src);
    let mut followed = None;
    if follow
        && s.location.is_some()
        && let Some(f) = crate::anchors::follow(src)
    {
        s = scan(src, Some(&f));
        followed = Some(f);
    }
    let mut c = Collector::new();
    // Every parent step of the file, for the strings that climb: each one's own
    // steps are taken off again.
    let raise = dots(&s.literals).saturating_add(s.parents);
    for l in &s.literals {
        collect(&mut c, l, raise, s.chdir || s.dir_in_loop);
    }
    // What can be the base of a climbing string of another file of the package, and
    // climb further than that file's parent steps say: a `Chdir`, a `filepath.Dir`
    // loop, the file's own location followed above its directory
    // (`filepath.Join(filepath.Dir(file), "..", "..")`; those parent steps are no
    // string of the file).
    c.names.moves_base |= s.chdir
        || s.dir_in_loop
        || followed
            .as_ref()
            .is_some_and(|f| f.entries.iter().any(|a| a.rel.starts_with("..")));
    // A call that changes the working directory (not into a temporary directory):
    // the relative paths of the directory's files can go anywhere.
    c.names.chdir = s.chdir && !chdir_calls(src).is_empty();
    let mut names = c.finish();
    names.ups = (names.ups + s.parents).min(MAX_UPS);
    match followed {
        Some(f) => names.anchored = f.entries,
        None => names.anchor = s.location,
    }
    names.cwd = s.getwd.or_else(|| test_file.then_some(1));
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lits(src: &str) -> Vec<(String, bool)> {
        strings(src.as_bytes())
            .literals
            .into_iter()
            .map(|l| (l.value, l.path_arg))
            .collect()
    }

    fn values(src: &str) -> Vec<String> {
        lits(src).into_iter().map(|l| l.0).collect()
    }

    #[test]
    fn literals_and_path_arguments() {
        let src = r#"package p

import (
	"os"
	fp "path/filepath"
)

const schema = "schemas/job.json"

func load(dir string) {
	os.ReadFile(fp.Join(dir, "fixtures", name))
	run("go", `raw/text.txt`)
}
"#;
        assert_eq!(
            lits(src),
            [
                ("schemas/job.json".to_owned(), false),
                ("fixtures".to_owned(), true),
                ("go".to_owned(), false),
                ("raw/text.txt".to_owned(), false),
            ]
        );
        // Consecutive literal arguments of a path function are one path; an argument
        // that is not a literal ends the run.
        assert_eq!(
            lits(
                "package p\nvar a = filepath.Join(root, \"src\", \"routes.ts\")\nvar b = \
                 filepath.Join(\"..\", \"..\", \"data\", name, \"x.json\", \"y\")\nvar c = \
                 f(\"a\", \"b\")\n"
            ),
            [
                ("src/routes.ts".to_owned(), true),
                ("../../data".to_owned(), true),
                ("x.json/y".to_owned(), true),
                ("a".to_owned(), false),
                ("b".to_owned(), false),
            ]
        );
        let s = strings(src.as_bytes());
        assert_eq!(s.literals[0].line, 8);
        assert_eq!(s.literals[1].line, 11);
    }

    #[test]
    fn format_verbs_are_holes() {
        let src = "package p\nvar a = fmt.Sprintf(\"configs/%s-%03d.yaml\", n, i)\nvar b = \
                   \"100%\"\nvar c = fmt.Sprintf(\"%d%%\", n)\n";
        assert_eq!(
            values(src),
            [
                format!("configs/{HOLE}-{HOLE}.yaml"),
                "100%".to_owned(),
                format!("{HOLE}%"),
            ]
        );
    }

    /// The names of `src` (a test file) by shape: whether the plain climbing
    /// mark is set and how far they climb from the working directory.
    fn climbing(src: &str) -> Vec<(String, bool, u32)> {
        names(src.as_bytes(), true)
            .names
            .into_iter()
            .map(|n| (n.shape.into_string(), n.climbs, n.climbs_from_cwd))
            .collect()
    }

    #[test]
    fn strings_that_climb_are_marked_with_how_far_they_climb() {
        let own = |s: &str, k: u32| (s.to_owned(), false, k);
        let free = |s: &str| (s.to_owned(), true, 0);
        // A literal that climbs four directories to a file.
        assert_eq!(
            climbing("package p\nvar a, _ = os.ReadFile(\"../../../../package.json\")\n"),
            [own("package.json", 4)]
        );
        // Consecutive literal arguments of `Join` are one string; a base below the
        // working directory spelled in the same call takes steps back. Each string
        // can also climb by the file's other parent steps.
        assert_eq!(
            climbing(
                "package p\nvar a = filepath.Join(\"..\", \"..\", \"shared\", name)\nvar b = \
                 filepath.Join(\"a/b\", \"../../../x/y.json\")\n"
            ),
            [own("shared", 5), own("x/y.json", 3)]
        );
        // A base in a variable: the string climbs as far as it says.
        assert_eq!(
            climbing(
                "package p\nvar a = filepath.Join(dir, \"../../../x/y.json\")\nvar dir = \
                 \"testdata/golden\"\n"
            ),
            [own("x/y.json", 3), own("testdata/golden", 0)]
        );
        // The file's `filepath.Dir` calls can raise the base: they count.
        assert_eq!(
            climbing("package p\nvar a = filepath.Join(filepath.Dir(wd), \"../x/y.json\")\n"),
            [own("x/y.json", 2)]
        );
        // In a helper they can run more than once: the plain mark.
        assert_eq!(
            climbing(
                "package p\nfunc f(wd string) string {\n\treturn filepath.Join(filepath.Dir(wd), \
                 \"../x/y.json\")\n}\n"
            ),
            [free("x/y.json")]
        );
        // The same shape without parent steps is one name; the largest climb is kept
        // (each string's own steps plus the file's others).
        assert_eq!(
            climbing(
                "package p\nvar a = []string{\"./services\", \"../services\"}\nvar b = \
                 filepath.Join(\"../../../services\")\n"
            ),
            [own("services", 4)]
        );
        // A rest that is not the shape keeps the plain mark: a step back after a named
        // segment, a hole before the name.
        assert_eq!(
            climbing(
                "package p\nvar a = \"../a/b/../c.json\"\nvar b = \
                 fmt.Sprintf(\"../%s/data/x.json\", n)\nvar c = \"../a/../d.json\"\n"
            ),
            [free("c.json"), free("data/x.json"), own("d.json", 4)]
        );
        // A plain mark wins over the working directory's.
        assert_eq!(
            climbing("package p\nvar a = []string{\"../d/e.json\", \"../z/y/../d/e.json\"}\n"),
            [free("d/e.json")]
        );
        // A file that changes its working directory: the plain mark.
        assert_eq!(
            climbing(
                "package p\nfunc TestX(t *testing.T) {\n\tt.Chdir(dir)\n\t_, _ = \
                 os.ReadFile(\"../../x/y.json\")\n}\n"
            ),
            [free("x/y.json")]
        );
        // The other parent steps of the file raise how far a string can climb.
        assert_eq!(
            climbing(
                "package p\nvar root = filepath.Join(\"..\", \"..\", name)\nvar a = \
                 filepath.Join(root, \"../x/y.json\")\n"
            ),
            [own("x/y.json", 3)]
        );
        // `filepath.Dir` in a loop: the plain mark.
        assert_eq!(
            climbing(
                "package p\nfunc f(d string) string {\n\tfor i := 0; i < n; i++ {\n\t\td = \
                 filepath.Dir(d)\n\t}\n\treturn filepath.Join(d, \"../x/y.json\")\n}\n"
            ),
            [free("x/y.json")]
        );
        // A loop whose header has a composite literal: its body is still the loop.
        let s = strings(
            b"package p\nfunc f(d string) {\n\tfor _, x := range []string{\"a\"} {\n\t\td = \
              filepath.Dir(d)\n\t}\n}\nfunc g(d string) string {\n\treturn filepath.Dir(d)\n}\n",
        );
        assert!(s.dir_in_loop);
        let repeats = |body: &str| strings(format!("package p\n{body}").as_bytes()).dir_in_loop;
        // Once: at package level, or directly in a test function, after a loop too.
        for body in [
            "var a = filepath.Dir(d)\n",
            "func TestG(t *testing.T) {\n\t_ = filepath.Dir(d)\n}\n",
            "func TestG(t *testing.T) {\n\tfor _, s := range []int{1} {\n\t\t_ = s\n\t}\n\td = \
             filepath.Dir(d)\n}\n",
            "func TestG(t *testing.T) {\n\tvar f func(string) string\n\t_ = f\n\tif ok \
             {\n\t}\n\td = filepath.Dir(d)\n}\n",
            "func init() {\n\td = filepath.Dir(d)\n}\n",
        ] {
            assert!(!repeats(body), "{body}");
        }
        // More than once: in a loop's clauses or body (after a composite literal in
        // its clauses, on one line or several), in a function literal, in a helper,
        // recursively, in a test function that other code calls, with `goto`.
        for body in [
            "func TestG(t *testing.T) {\n\tfor d := wd; d != filepath.Dir(d); d = filepath.Dir(d) \
             {\n\t}\n}\n",
            "func TestG(t *testing.T) {\n\tfor _, s := range []int{1, 2} {\n\t\td = \
             filepath.Dir(d)\n\t}\n}\n",
            "func TestG(t *testing.T) {\n\tfor _, s := range []struct{ n int }{\n\t\t{1}, \
             {2},\n\t} {\n\t\t_ = s\n\t\td = filepath.Dir(d)\n\t}\n}\n",
            "func TestG(t *testing.T) {\n\tfor _, s := range map[string]int{\n\t\t\"a\": 1,\n\t} \
             {\n\t\td = filepath.Dir(d)\n\t}\n}\n",
            "func TestG(t *testing.T) {\n\tup := func(p string) string { return filepath.Dir(p) \
             }\n\t_ = up\n}\n",
            "func up(p string) string {\n\tif p != \"\" {\n\t\treturn \
             up(filepath.Dir(p))\n\t}\n\treturn p\n}\n",
            "func TestA(t *testing.T) {\n\td = filepath.Dir(d)\n}\nfunc TestB(t *testing.T) \
             {\n\tTestA(t)\n}\n",
            "func TestG(t *testing.T) {\nL:\n\td = filepath.Dir(d)\n\tgoto L\n}\n",
        ] {
            assert!(repeats(body), "{body}");
        }
        // Parent steps through an alias, a dot import, and `Split`.
        for (src, n) in [
            (
                "package p\nimport fp \"path/filepath\"\nvar a = fp.Dir(x)\n",
                1,
            ),
            (
                "package p\nimport (\n\t. \"path/filepath\"\n)\nvar a = Dir(x)\n",
                1,
            ),
            (
                "package p\nvar a, _ = filepath.Split(x)\nvar b = strings.Split(x, \"/\")\n",
                1,
            ),
            ("package p\nvar a = other.Dir(x)\n", 0),
        ] {
            assert_eq!(strings(src.as_bytes()).parents, n, "{src}");
        }
        // A string that only climbs, or a `Chdir`, marks the file for its package.
        for src in [
            "package p\nconst Root = \"../..\"\n",
            "package p\nvar r = filepath.Join(\"..\", \"..\")\n",
            "package p\nvar r, _ = filepath.Abs(\"../..\")\n",
            "package p\nfunc TestMain(m *testing.M) {\n\tos.Chdir(dir)\n}\n",
            // A loop of `filepath.Dir` calls, and the file's location followed above its
            // directory: a base of the other files' strings that climbs further.
            "package p\nfunc root(d string) string {\n\tfor {\n\t\td = filepath.Dir(d)\n\t}\n}\n",
            "package p\nfunc root() string {\n\t_, file, _, _ := runtime.Caller(0)\n\treturn \
             filepath.Join(filepath.Dir(file), \"..\", \"..\")\n}\n",
        ] {
            let n = names(src.as_bytes(), false);
            assert!(n.moves_base, "{src}");
            assert!(!n.is_empty(), "{src}");
        }
        assert!(!names(b"package p\nvar a = \"../x/y.json\"\n", false).moves_base);
        // No parent steps, no mark; a literal path from the location is no climb.
        assert_eq!(
            climbing(
                r#"package p
func specFile() string {
	_, self, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(self), "..", "..", "specs", "order.json")
}
var x = "fixtures/a.json"
"#
            ),
            [own("specs/order.json", 0), own("fixtures/a.json", 0)]
        );
    }

    #[test]
    fn calls_that_change_the_working_directory() {
        let calls = |src: &str| chdir_calls(src.as_bytes()).len();
        assert_eq!(
            calls("package p\nfunc TestMain(m *testing.M) {\n\tos.Chdir(\"../..\")\n}\n"),
            1
        );
        assert_eq!(
            calls("package p\nfunc TestX(t *testing.T) {\n\tt.Chdir(dir)\n}\n"),
            1
        );
        // Into a temporary directory: what is read there is no repository file.
        assert_eq!(
            calls(
                "package p\nfunc TestX(t *testing.T) {\n\tt.Chdir(t.TempDir())\n\tdir := \
                 t.TempDir()\n\tos.Chdir(dir)\n}\n"
            ),
            0
        );
        // A declaration is no call.
        assert_eq!(
            calls(
                "package p\nfunc Chdir(dir string) error { return nil }\nfunc (f *fs) Chdir(dir \
                 string) error { return nil }\n"
            ),
            0
        );
        let n = names(
            b"package p\nfunc TestX(t *testing.T) {\n\tt.Chdir(dir)\n}\n",
            true,
        );
        assert!(n.chdir && n.moves_base);
        let n = names(
            b"package p\nfunc TestX(t *testing.T) {\n\tt.Chdir(t.TempDir())\n}\n",
            true,
        );
        assert!(!n.chdir && n.moves_base);
    }

    #[test]
    fn messages_are_not_collected() {
        let src = r#"package p
func TestX(t *testing.T) {
	t.Fatalf("read config.json: %v", err)
	t.Log("wrote out/report.txt")
	_ = fmt.Errorf("open data/a.bin: %w", err)
	_ = errors.New("missing settings.yaml")
	x := template.New("page.tmpl")
	panic("bad state.db")
}
"#;
        assert_eq!(values(src), ["page.tmpl"]);
    }

    #[test]
    fn anchors_and_parent_steps() {
        let src = r#"package p
func root() string {
	_, file, _, _ := runtime.Caller(0)
	wd, _ := os.Getwd()
	return filepath.Join(filepath.Dir(filepath.Dir(file)), wd)
}
"#;
        // The location is used where the file variable is, not where it is declared.
        let s = strings(src.as_bytes());
        assert_eq!((s.location, s.getwd, s.parents), (Some(5), Some(4), 2));
        let s = strings(b"package p\nvar x = y.Caller(0)\n");
        assert_eq!((s.location, s.getwd, s.parents), (None, None, 0));
        // A call whose file is not bound is a use by itself.
        let s = strings(b"package p\nfunc f() {\n\tg(runtime.Caller(0))\n}\n");
        assert_eq!(s.location, Some(3));
    }

    #[test]
    fn a_literal_path_from_the_location_is_not_a_use_of_it() {
        // A helper that returns a file two directories up.
        let src = r#"package p
func specFile(t *testing.T) string {
	_, self, _, found := runtime.Caller(0)
	if !found {
		t.Fatal("caller unknown")
	}
	return filepath.Join(filepath.Dir(self), "..", "..", "specs", "order.json")
}
"#;
        let s = strings(src.as_bytes());
        assert_eq!((s.location, s.parents), (None, 0));
        // The parent steps are gone from the literal: they move nothing else.
        assert_eq!(values(src), ["specs/order.json"]);
        // A path that only goes up, names a directory, or continues with something
        // unknown, uses the location.
        let dir = r#"package p
func services() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..", "..", "services")
}
"#;
        let s = strings(dir.as_bytes());
        assert_eq!((s.location, s.parents), (Some(4), 1));
        assert_eq!(values(dir), ["../../services"]);
        let up = r#"package p
func repoRoot() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", ".."))
}
"#;
        let s = strings(up.as_bytes());
        assert_eq!((s.location, s.parents), (Some(4), 1));
        assert_eq!(values(up), ["../.."]);
        let open = r#"package p
func ddl(name string) string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Join(filepath.Dir(file), "..", "warehouse", name)
}
"#;
        let s = strings(open.as_bytes());
        assert_eq!((s.location, s.parents), (Some(4), 1));
        assert_eq!(values(open), ["../warehouse"]);
    }

    #[test]
    fn arguments_that_only_test_or_cut_a_string_are_not_collected() {
        let src = r#"package p
func f(routes string) {
	dir := strings.TrimSuffix(filepath.Dir(routes), "/src")
	if strings.HasPrefix(routes, "testdata/") || strings.Contains(routes, "/fixtures/") {
	}
	base := strings.TrimSuffix("configs/app.yaml", ".yaml")
	out := strings.ReplaceAll(dir, "src", "dist/out")
}
"#;
        assert_eq!(values(src), ["configs/app.yaml", "src", "dist/out"]);
    }

    #[test]
    fn strings_below_a_temp_root_are_not_collected() {
        let src = r#"package p
func TestA(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "config.json"), nil, 0o600)
	os.ReadFile(filepath.Join(t.TempDir(), "settings.yaml"))
	os.ReadFile(filepath.Join(fixtures, "golden.json"))
	os.ReadFile(filepath.Join(dir, sub, "deep.txt"))
}
"#;
        assert_eq!(values(src), ["golden.json"]);
    }
}
