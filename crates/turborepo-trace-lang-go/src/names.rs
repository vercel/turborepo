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
//! | [`Strings::parents`] | `filepath.Dir` / `path.Dir` calls: steps toward the root (`..` segments are counted from the strings) |
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
//! joined onto. The query names the rest at those directories, in whatever
//! package they are, the rest completed with an extension there too
//! (`"../../shared/golden" + ".json"`), and the shape anywhere below the
//! working directory and inside the package like any other string
//! (`turborepo_trace_graph::names`, "Strings that climb from the working
//! directory"). This assumes:
//!
//! > A Go string that climbs with parent steps is taken from the working
//! > directory the test starts in, or from a directory below it: it is not
//! > joined onto another path that climbs above the working directory or leads
//! > elsewhere (a directory that another package finds from its own location,
//! > the value of an environment variable), and the process does not change its
//! > working directory (`os.Chdir`, also in the `init` of an imported package).
//!
//! The file's own location names its own places ([`crate::anchors`]), and the
//! consecutive literal arguments of one `filepath.Join` call are one string.
//! What the analysis can see of a further climb keeps the plain climbing mark,
//! whose shape names anywhere:
//!
//! - a string whose rest is not its shape: a named segment left before a later
//!   parent step (`"../a/b/../c"`) or a hole before the name (`"../%s/data"`);
//! - every climbing string of a file that calls `filepath.Dir` in a loop;
//! - every climbing string of a package (the files of one directory) in which a
//!   file calls `Chdir` (`os.Chdir`, `t.Chdir`, a `TestMain` that moves) or
//!   holds a string that only climbs (`"../.."`, `const Root = "../.."`,
//!   `filepath.Join("..", "..")`): such a string can be the base of the others
//!   ([`Names::moves_base`](turborepo_trace_core::names::Names::moves_base)).
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
    /// A counted `filepath.Dir` / `path.Dir` call is inside a `for` loop: it
    /// can run any number of times, so [`Strings::parents`] does not bound
    /// how far it climbs.
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
    /// `{` opened after a `for` keyword: the loop's body (or a composite
    /// literal of its header, which is taken as part of it).
    looped: bool,
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
    // A `for` keyword whose body has not been opened yet.
    let mut for_pending = false;
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
                        looped: false,
                    });
                }
                b'{' => {
                    let looped = for_pending;
                    // The body of a loop ends its line (gofmt); a composite literal of the
                    // header does not, and the body is still to come.
                    if for_pending
                        && next(1).is_none_or(|n| {
                            n.is_punct(src, b'}')
                                || memchr::memchr(b'\n', &src[at..n.span.start as usize]).is_some()
                        })
                    {
                        for_pending = false;
                    }
                    groups.push(Group {
                        looped,
                        ..Group::default()
                    });
                }
                b'[' => groups.push(Group::default()),
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
                    b"for" if !prev(1).is_some_and(|p| p.is_punct(src, b'.')) => {
                        for_pending = true;
                    }
                    b"Dir"
                        if !in_literal
                            && !followed.is_some_and(|f| part(&f.dirs, at))
                            && (after(b"filepath") || after(b"path"))
                            && next(1).is_some_and(|n| n.is_punct(src, b'(')) =>
                    {
                        out.parents = (out.parents + 1).min(MAX_PARENTS);
                        if groups.iter().any(|g| g.looped) {
                            out.dir_in_loop = true;
                        }
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
    out
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
    c.names.moves_base |= s.chdir;
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
            climbing(
                "package p\nfunc f(wd string) string {\n\treturn filepath.Join(filepath.Dir(wd), \
                 \"../x/y.json\")\n}\n"
            ),
            [own("x/y.json", 2)]
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
        let s = strings(b"package p\nfunc g(d string) string {\n\treturn filepath.Dir(d)\n}\n");
        assert!(!s.dir_in_loop);
        // A string that only climbs, or a `Chdir`, marks the file for its package.
        for src in [
            "package p\nconst Root = \"../..\"\n",
            "package p\nvar r = filepath.Join(\"..\", \"..\")\n",
            "package p\nvar r, _ = filepath.Abs(\"../..\")\n",
            "package p\nfunc TestMain(m *testing.M) {\n\tos.Chdir(dir)\n}\n",
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
