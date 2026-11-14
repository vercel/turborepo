//! Where a Go file's own location goes: the anchored names of a file
//! (`turborepo_trace_core::names::Anchored`).
//!
//! The token scan ([`crate::names`]) knows that a file uses the file
//! `runtime.Caller` reports other than in a literal path to a file, and has to
//! assume the worst: the file can produce any path below its directory, raised
//! by every `..` and `filepath.Dir` of the file. For a test harness that finds
//! the repository root that way, this is every path of the repository. This
//! module follows the located value through the file instead ([`follow`]), as
//! `turborepo_trace_lang_js::flow::anchors` does for `__dirname`:
//!
//! 1. the *value*: the file (`_, file, _, _ := runtime.Caller(0)`), and what
//!    `filepath.Dir`, `filepath.Join` with literal segments, `filepath.Clean`,
//!    `filepath.Abs`, `filepath.EvalSymlinks`, `filepath.ToSlash` / `FromSlash`
//!    and `+` with string literals make of it: a directory relative to the
//!    file's own. A part that is not a literal makes the value *open*: a path
//!    at or below the directory known so far;
//! 2. what consumes the largest such expression:
//!    - the definition of a variable of the function (`root := …`, `root = …`,
//!      `real, err := filepath.EvalSymlinks(root)`): the value of a name is the
//!      join of its definitions in the function, and every use of the name in
//!      the function is followed in turn (no flow or block order: a name
//!      declared in a function is one variable, closures included);
//!    - an argument of a function declared in this file (`sweep(ctx, root)`):
//!      the parameter gets the value, and its uses in that function are
//!      followed;
//!    - the base of `filepath.Rel`, an argument that is only tested or measured
//!      (`strings.HasPrefix(p, root)`, `filepath.Base(root)`), a message
//!      (`t.Logf`, `fmt.Errorf`, `panic`), a comparison, a `case`: not a read,
//!      nothing is named;
//!    - the working directory of a child process (`cmd.Dir = root` with every
//!      definition of `cmd` an `exec.Command` or `exec.CommandContext` call,
//!      and `cmd` used only through its fields and methods): this process reads
//!      nothing there. With a command line of string literals, the words of it
//!      that are paths (`"./scripts/gen.sh"`, `"--config=configs/a.json"`) are
//!      named at that directory; with any other part the directory escapes;
//!    - anything else *escapes*: an argument of a function this file does not
//!      declare (a file API included: the path it reads is at or below the
//!      value), a composite literal, a struct field, a map or slice, a channel,
//!      and whatever reaches package scope: the result of a function or method
//!      (`return root`), a named result, a package variable. The file then
//!      names everything at or below the directory the value has there.
//!
//! A use that cannot be followed makes [`follow`] return `None`, and the file
//! keeps its coarse mark: a `runtime.Caller` with a skip other than `0` (the
//! caller's file), a program counter that is kept, a call outside a function
//! declaration, the file itself handed on (`return file`: the first parent step
//! of whoever gets it only removes the file name, which the coarse rule knows
//! and an entry cannot say), a located name that is sliced, indexed, has its
//! address taken or is extended in place (`root +=`), a `strings` function that
//! cuts or rewrites it, a located path joined below another one, more than a
//! few values per name.
//!
//! # Why the result of a function escapes
//!
//! A Go package is a directory: a function, a struct field and a package
//! variable are visible in every file of it, whatever their case, and
//! extraction sees one file. So `func repoRoot() string` that returns the root
//! is a value other files of the package can read, and it escapes where it is
//! returned. Only what stays inside function bodies of this file is followed.
//! (Joining the files of a package would need a fact per file and a step at
//! query time.)
//!
//! # Assumptions
//!
//! As for the token scan and for the JS follower: an unknown part of a path
//! does not climb (`filepath.Join(root, name)` stays below `root`; the parent
//! steps the file has elsewhere are applied to every entry at query time), code
//! that receives a path does not cut it by other means than `filepath.Dir` and
//! `..`, and what a message or an error says is not parsed back into a path. A
//! child process is not this process: the files a command reads in its working
//! directory are not named (its literal path arguments are). A name declared
//! anywhere in a function is one variable of that function: a package
//! variable that an inner block hides under the same name is not seen as one.
//! `filepath`, `path`, `strings`, `fmt`, `exec` and `runtime` are taken by
//! name, like the path functions of the token scan.

use std::collections::{HashMap, HashSet};

use turborepo_trace_core::names::{Anchored, EVERYTHING_BELOW, MAX_UPS, NameKind};

use crate::lex::{Kind, Tok, string_value, tokenize};

/// Most expressions followed per file.
const MAX_STEPS: u32 = 4096;
/// Most values of one name.
const MAX_VALUES: usize = 8;
/// Most entries of one file.
const MAX_ENTRIES: usize = 64;
/// Most passes over the file until the values of the names are stable.
const MAX_ROUNDS: usize = 16;

/// What following the location of a file found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Followed {
    /// Where the location goes.
    pub entries: Vec<Anchored>,
    /// Byte offsets of the string literals whose parent steps are part of the
    /// entries.
    pub literals: Vec<u32>,
    /// Byte offsets of the `Dir` of the `filepath.Dir` calls that are part of
    /// the entries.
    pub dirs: Vec<u32>,
}

/// A path relative to the directory of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Loc {
    /// Parent steps above the file's directory.
    ups: u32,
    /// Segments below that.
    down: Vec<String>,
    /// Something unknown follows: the path is at or below `ups` / `down`.
    open: bool,
    /// The file itself (`down` and `ups` are its directory).
    file: bool,
    /// Something was appended to the file itself (`file + ".bak"`, `Join(file,
    /// name)`): not a path the follower places.
    below_file: bool,
}

impl Loc {
    const FILE: Loc = Loc {
        ups: 0,
        down: Vec::new(),
        open: false,
        file: true,
        below_file: false,
    };

    /// One parent step. For an open path the known directory is raised: with an
    /// empty unknown part the step applies to it.
    fn up(&mut self) {
        if self.file {
            self.file = false;
        } else if self.down.pop().is_none() {
            self.ups += 1;
        }
    }

    /// One more segment.
    fn seg(&mut self, s: &str) {
        match s {
            "" | "." => {}
            ".." => self.up(),
            _ if self.file => {
                self.file = false;
                self.below_file = true;
                self.open = true;
            }
            _ if self.open => {}
            _ => self.down.push(s.to_owned()),
        }
    }

    /// An unknown segment follows.
    fn unknown(&mut self) {
        self.below_file |= self.file;
        self.file = false;
        self.open = true;
    }

    /// Unknown text is appended to the last segment (`root + "-old"`): a
    /// sibling of it.
    fn partial(&mut self) {
        if self.file {
            self.file = false;
            self.below_file = true;
        } else if !self.open {
            self.up();
        }
        self.open = true;
    }

    /// The literal `s` is appended as text. `sep`: the path so far ends with a
    /// separator; returns the same for the result.
    fn text(&mut self, s: &str, sep: bool) -> bool {
        if s.is_empty() {
            return sep;
        }
        let mut chunks = s.split('/');
        let first = chunks.next().unwrap_or("");
        if !first.is_empty() {
            if sep {
                self.seg(first);
            } else {
                self.partial();
            }
        }
        for c in chunks {
            self.seg(c);
        }
        s.ends_with('/')
    }

    /// The directory, relative to the file's: `""`, `../..`, `../../services`.
    fn rel(&self) -> String {
        let mut out: Vec<&str> = vec![".."; self.ups as usize];
        out.extend(self.down.iter().map(String::as_str));
        out.join("/")
    }
}

/// The follower gave up: the file keeps its coarse mark.
struct Coarse;

type Step<T = ()> = Result<T, Coarse>;

/// A function declaration of the file.
struct Func<'s> {
    /// Parameter names by position (`None`: unnamed or not a plain name) and
    /// whether the parameter is variadic.
    params: Vec<(Option<&'s [u8]>, bool)>,
    /// Named results.
    results: Vec<&'s [u8]>,
    /// The braces of the body (indices of significant tokens).
    body: (usize, usize),
    /// Names declared in the body, the parameters and the results.
    declared: HashSet<&'s [u8]>,
}

/// The callee of a call.
enum Callee<'s> {
    /// `name(`.
    Bare(&'s [u8]),
    /// `recv.name(`: both names and the token of the receiver.
    Qualified(&'s [u8], &'s [u8], usize),
}

/// The `=` of a statement or an expression.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Eq {
    /// `:=`
    Define,
    /// `=`
    Assign,
    /// `+=` and the like.
    Op,
    /// `==`, `!=`, `<=`, `>=`.
    Compare,
}

/// What an expression that is one element of a comma list belongs to.
enum Context {
    /// An argument of the call whose `(` is at the index.
    Call(usize),
    /// An element of a bracket that is not a call: a composite literal, an
    /// index.
    Group,
    /// The right side of the assignment whose `=` is at the index.
    Assign(usize, Eq),
    /// A `return`.
    Return,
    /// A `case`.
    Case,
    /// A parameter list of a function literal: a declaration.
    Decl,
    /// Anything else.
    Other,
}

const NONE: usize = usize::MAX;

struct Walk<'s> {
    src: &'s [u8],
    /// The significant tokens.
    toks: Vec<Tok>,
    /// A newline lies before the token.
    nl: Vec<bool>,
    /// The matching bracket of each bracket.
    mate: Vec<usize>,
    funcs: Vec<Func<'s>>,
    /// The functions (not methods) by name.
    by_name: HashMap<&'s [u8], usize>,
    /// Per function: the located names and their values.
    vals: Vec<HashMap<&'s [u8], Vec<Loc>>>,
    /// Entries: byte offset and directory.
    entries: Vec<(u32, String)>,
    literals: Vec<u32>,
    dirs: Vec<u32>,
    changed: bool,
    steps: u32,
}

fn keyword(w: &[u8]) -> bool {
    matches!(
        w,
        b"return"
            | b"if"
            | b"for"
            | b"switch"
            | b"case"
            | b"go"
            | b"defer"
            | b"range"
            | b"else"
            | b"var"
            | b"const"
            | b"func"
            | b"select"
            | b"default"
            | b"type"
            | b"break"
            | b"continue"
            | b"goto"
            | b"fallthrough"
            | b"chan"
            | b"map"
            | b"struct"
            | b"interface"
            | b"import"
            | b"package"
    )
}

/// `strings` / `bytes` functions whose result is a truth value or a number.
fn measures(name: &[u8]) -> bool {
    matches!(
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
    )
}

/// `strings` / `bytes` functions that cut their first argument with the others.
fn cuts(name: &[u8]) -> bool {
    matches!(
        name,
        b"TrimPrefix"
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

impl<'s> Walk<'s> {
    fn new(src: &'s [u8]) -> Self {
        let (all, _) = tokenize(src);
        let mut toks = Vec::with_capacity(all.len());
        let mut nl = Vec::with_capacity(all.len());
        let mut at = 0usize;
        for t in all {
            if t.is_comment() {
                continue;
            }
            let start = t.span.start as usize;
            nl.push(memchr::memchr(b'\n', &src[at.min(start)..start]).is_some());
            at = t.span.end as usize;
            toks.push(t);
        }
        let mut mate = vec![NONE; toks.len()];
        let mut stack: Vec<usize> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            if t.kind != Kind::Punct {
                continue;
            }
            match src[t.span.start as usize] {
                b'(' | b'[' | b'{' => stack.push(i),
                c @ (b')' | b']' | b'}') => {
                    let open = match c {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    if let Some(&o) = stack.last()
                        && src[toks[o].span.start as usize] == open
                    {
                        stack.pop();
                        mate[o] = i;
                        mate[i] = o;
                    }
                }
                _ => {}
            }
        }
        Walk {
            src,
            toks,
            nl,
            mate,
            funcs: Vec::new(),
            by_name: HashMap::new(),
            vals: Vec::new(),
            entries: Vec::new(),
            literals: Vec::new(),
            dirs: Vec::new(),
            changed: false,
            steps: 0,
        }
    }

    fn p(&self, i: usize, b: u8) -> bool {
        self.toks.get(i).is_some_and(|t| t.is_punct(self.src, b))
    }

    fn id(&self, i: usize) -> Option<&'s [u8]> {
        self.toks
            .get(i)
            .filter(|t| t.kind == Kind::Ident)
            .map(|t| t.text(self.src))
    }

    fn word(&self, i: usize, w: &[u8]) -> bool {
        self.id(i) == Some(w)
    }

    fn kw(&self, i: usize) -> bool {
        self.id(i).is_some_and(keyword)
    }

    fn opener(&self, i: usize) -> bool {
        self.p(i, b'(') || self.p(i, b'[') || self.p(i, b'{')
    }

    fn closer(&self, i: usize) -> bool {
        self.p(i, b')') || self.p(i, b']') || self.p(i, b'}')
    }

    /// Whether the tokens `i` and `i + 1` touch.
    fn adjacent(&self, i: usize) -> bool {
        match (self.toks.get(i), self.toks.get(i + 1)) {
            (Some(a), Some(b)) => a.span.end == b.span.start,
            _ => false,
        }
    }

    /// Whether a newline after the token `i` ends a statement.
    fn can_end(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(|t| match t.kind {
            Kind::Ident | Kind::Str | Kind::RawStr | Kind::Rune | Kind::Number => true,
            Kind::Punct => self.closer(i),
            _ => false,
        })
    }

    /// Whether a statement starts at the token `i`.
    fn starts(&self, i: usize) -> bool {
        i == 0 || (self.nl[i] && self.can_end(i - 1)) || self.p(i - 1, b';')
    }

    /// What the `=` at `i` is.
    fn eq(&self, i: usize) -> Eq {
        if self.p(i + 1, b'=') && self.adjacent(i) {
            return Eq::Compare;
        }
        if i == 0 || !self.adjacent(i - 1) {
            return Eq::Assign;
        }
        let prev = &self.toks[i - 1];
        if prev.kind != Kind::Punct {
            return Eq::Assign;
        }
        match self.src[prev.span.start as usize] {
            b'=' | b'!' | b'<' | b'>' => Eq::Compare,
            b':' => Eq::Define,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' => Eq::Op,
            _ => Eq::Assign,
        }
    }

    /// The top-level elements of the bracket `open`: token ranges.
    fn elements(&self, open: usize) -> Vec<(usize, usize)> {
        let close = self.mate[open];
        let mut out = Vec::new();
        if close == NONE {
            return out;
        }
        let mut start = open + 1;
        let mut k = open + 1;
        while k < close {
            if self.opener(k) && self.mate[k] != NONE {
                k = self.mate[k] + 1;
                continue;
            }
            if self.p(k, b',') {
                out.push((start, k));
                start = k + 1;
            }
            k += 1;
        }
        if start < close {
            out.push((start, close));
        }
        out
    }

    /// The names of a parameter or result list.
    fn params(&self, open: usize) -> Vec<(Option<&'s [u8]>, bool)> {
        self.elements(open)
            .into_iter()
            .map(|(a, b)| {
                let name = self
                    .id(a)
                    .filter(|w| !keyword(w) && (b - a == 1 || !self.p(a + 1, b'.')));
                let variadic = (a..b).any(|k| self.p(k, b'.') && self.p(k + 1, b'.'));
                (name, variadic)
            })
            .collect()
    }

    /// The function declarations of the file.
    fn declarations(&mut self) {
        let n = self.toks.len();
        let mut i = 0;
        while i < n {
            if self.opener(i) && self.mate[i] != NONE {
                i = self.mate[i] + 1;
                continue;
            }
            if !self.word(i, b"func") {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            let method = self.p(j, b'(');
            if method {
                if self.mate[j] == NONE {
                    break;
                }
                j = self.mate[j] + 1;
            }
            let Some(name) = self.id(j) else {
                i += 1;
                continue;
            };
            j += 1;
            // Type parameters.
            if self.p(j, b'[') && self.mate[j] != NONE {
                j = self.mate[j] + 1;
            }
            if !self.p(j, b'(') || self.mate[j] == NONE {
                i += 1;
                continue;
            }
            let params = self.params(j);
            j = self.mate[j] + 1;
            let mut results = Vec::new();
            if self.p(j, b'(') && self.mate[j] != NONE {
                for (a, b) in self.elements(j) {
                    if b - a >= 2
                        && !self.p(a + 1, b'.')
                        && let Some(w) = self.id(a).filter(|w| !keyword(w))
                    {
                        results.push(w);
                    }
                }
                j = self.mate[j] + 1;
            }
            // The body: the first brace that is not the one of `interface{}` / `struct{}`
            // in the result type.
            let mut body = None;
            while j < n {
                if self.p(j, b'{') {
                    if self.mate[j] == NONE {
                        break;
                    }
                    if j > 0 && (self.word(j - 1, b"interface") || self.word(j - 1, b"struct")) {
                        j = self.mate[j] + 1;
                        continue;
                    }
                    body = Some((j, self.mate[j]));
                    break;
                }
                if self.starts(j) && j > i + 1 && !self.p(j - 1, b';') {
                    // A declaration without a body.
                    break;
                }
                if (self.p(j, b'(') || self.p(j, b'[')) && self.mate[j] != NONE {
                    j = self.mate[j] + 1;
                    continue;
                }
                j += 1;
            }
            let Some(body) = body else {
                i += 1;
                continue;
            };
            let mut declared: HashSet<&'s [u8]> = HashSet::new();
            declared.extend(params.iter().filter_map(|p| p.0));
            declared.extend(results.iter().copied());
            for k in body.0 + 1..body.1 {
                // `a, b := …`
                if self.p(k, b'=') && self.eq(k) == Eq::Define {
                    let mut t = k - 1;
                    while t > body.0 + 1 {
                        let Some(w) = self.id(t - 1) else { break };
                        declared.insert(w);
                        if t >= 2 && self.p(t - 2, b',') {
                            t -= 2;
                        } else {
                            break;
                        }
                    }
                }
                // `var a, b …`
                if self.word(k, b"var") {
                    let mut t = k + 1;
                    while let Some(w) = self.id(t) {
                        declared.insert(w);
                        if self.p(t + 1, b',') {
                            t += 2;
                        } else {
                            break;
                        }
                    }
                }
                // The parameters of a function literal.
                if self.word(k, b"func") && self.p(k + 1, b'(') {
                    declared.extend(self.params(k + 1).iter().filter_map(|p| p.0));
                }
            }
            if !method {
                self.by_name.insert(name, self.funcs.len());
            }
            self.funcs.push(Func {
                params,
                results,
                body,
                declared,
            });
            self.vals.push(HashMap::new());
            i = body.1 + 1;
        }
    }

    /// The function whose body holds the token `i`.
    fn func_at(&self, i: usize) -> Option<usize> {
        self.funcs.iter().position(|f| f.body.0 < i && i < f.body.1)
    }

    /// The name `name` of the function `f` also has the values `vals`.
    fn add(&mut self, f: usize, name: &'s [u8], vals: &[Loc]) -> Step {
        let have = self.vals[f].entry(name).or_default();
        for v in vals {
            if !have.contains(v) {
                if have.len() >= MAX_VALUES {
                    return Err(Coarse);
                }
                have.push(v.clone());
                self.changed = true;
            }
        }
        Ok(())
    }

    /// Everything at or below the directory of each value is named. The file
    /// itself is not followed further: the first parent step that whoever
    /// gets it applies only removes the file name, which the coarse rule
    /// knows and an entry cannot say.
    fn escape(&mut self, at: usize, vals: &[Loc]) -> Step {
        if vals.iter().any(|v| v.file || v.below_file) {
            return Err(Coarse);
        }
        for v in vals {
            self.entry(at, v.rel())?;
        }
        Ok(())
    }

    fn entry(&mut self, at: usize, rel: String) -> Step {
        if rel.chars().any(char::is_control) {
            return Err(Coarse);
        }
        if self.entries.iter().any(|e| e.1 == rel) {
            return Ok(());
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Coarse);
        }
        self.entries.push((self.toks[at].span.start, rel));
        Ok(())
    }

    /// The callee of the call whose `(` is at `open`.
    fn callee(&self, open: usize) -> Option<Callee<'s>> {
        if open == 0 {
            return None;
        }
        let name = self.id(open - 1).filter(|w| !keyword(w))?;
        if open >= 2 && self.p(open - 2, b'.') {
            let recv = self.id(open.checked_sub(3)?)?;
            if open >= 4 && self.p(open - 4, b'.') {
                return None;
            }
            return Some(Callee::Qualified(recv, name, open - 3));
        }
        Some(Callee::Bare(name))
    }

    /// Whether the `(` at `open` groups an expression: it follows no name and
    /// no bracket.
    fn grouping(&self, open: usize) -> bool {
        open == 0
            || !(self.closer(open - 1)
                || self
                    .id(open - 1)
                    .is_some_and(|w| !keyword(w) || w == b"func"))
    }

    /// What the comma-list element that starts at `a` belongs to, and its
    /// index.
    fn context(&self, a: usize) -> (Context, usize) {
        let mut k = a;
        let mut index = 0;
        loop {
            if self.starts(k) {
                return (Context::Other, index);
            }
            let t = k - 1;
            if self.closer(t) {
                if self.mate[t] == NONE {
                    return (Context::Other, index);
                }
                k = self.mate[t];
                continue;
            }
            if self.p(t, b',') {
                index += 1;
                k = t;
                continue;
            }
            if self.opener(t) {
                if self.p(t, b'(') {
                    if t > 0 && self.word(t - 1, b"func") {
                        return (Context::Decl, index);
                    }
                    if self.callee(t).is_some() {
                        return (Context::Call(t), index);
                    }
                }
                return (Context::Group, index);
            }
            if self.p(t, b'=') {
                match self.eq(t) {
                    Eq::Compare => {}
                    e => return (Context::Assign(t, e), index),
                }
            }
            if let Some(w) = self.id(t).filter(|w| keyword(w)) {
                return match w {
                    b"return" => (Context::Return, index),
                    b"case" => (Context::Case, index),
                    _ => (Context::Other, index),
                };
            }
            k = t;
        }
    }

    /// The targets of the assignment whose `=` is at `eq`: token ranges.
    fn targets(&self, eq: usize, kind: Eq) -> Vec<(usize, usize)> {
        let end = if kind == Eq::Define { eq - 1 } else { eq };
        let mut out = Vec::new();
        let mut k = end;
        let mut last = end;
        loop {
            if self.starts(k) {
                break;
            }
            let t = k - 1;
            if self.closer(t) {
                if self.mate[t] == NONE {
                    break;
                }
                k = self.mate[t];
                continue;
            }
            if self.p(t, b',') {
                out.push((k, last));
                last = t;
                k = t;
                continue;
            }
            if self.opener(t) || self.kw(t) {
                break;
            }
            k = t;
        }
        out.push((k, last));
        out.reverse();
        out
    }

    /// How many elements follow the element that ends before `b` on the right
    /// side of an assignment.
    fn elements_after(&self, b: usize) -> usize {
        let mut k = b;
        let mut count = 0;
        while k < self.toks.len() {
            if k > 0 && self.nl[k] && self.can_end(k - 1) {
                break;
            }
            if self.p(k, b',') {
                count += 1;
            } else if self.p(k, b';') || self.closer(k) || self.p(k, b'{') {
                break;
            } else if self.opener(k) {
                if self.mate[k] == NONE {
                    break;
                }
                k = self.mate[k];
            }
            k += 1;
        }
        count
    }

    /// Whether the name at `i` is a target of an assignment (`root, err := …`),
    /// and not a use. A name that is extended in place cannot be followed.
    fn target(&self, i: usize) -> Step<bool> {
        if self.p(i + 1, b'+') && self.p(i + 2, b'=') && self.adjacent(i + 1) {
            return Err(Coarse);
        }
        let mut k = i + 1;
        while self.p(k, b',') && self.id(k + 1).is_some() {
            k += 2;
        }
        let define = self.p(k, b':') && self.p(k + 1, b'=') && self.adjacent(k);
        Ok(define || (self.p(k, b'=') && self.eq(k) == Eq::Assign))
    }

    /// Whether the key at `i` (`{root: …`) is the name of a field: its braces
    /// follow a type name that is not the value type of a map
    /// (`harness{root: root}`, the cases of `switch x {`). A key of a map
    /// is a value that can be read back.
    fn field_name(&self, i: usize) -> bool {
        let mut k = i;
        let open = loop {
            if k == 0 {
                return false;
            }
            let t = k - 1;
            if self.closer(t) {
                if self.mate[t] == NONE {
                    return false;
                }
                k = self.mate[t];
                continue;
            }
            if self.p(t, b'{') {
                break t;
            }
            if self.opener(t) {
                return false;
            }
            k = t;
        };
        open > 0
            && self.id(open - 1).is_some_and(|w| !keyword(w))
            && !(open.saturating_sub(8)..open).any(|j| self.word(j, b"map"))
    }

    /// The single string literal of the tokens `[a, b)`.
    fn literal(&self, a: usize, b: usize) -> Option<(String, u32)> {
        let t = self.toks.get(a).filter(|t| b == a + 1 && t.is_string())?;
        Some((string_value(self.src, t)?.into_owned(), t.span.start))
    }

    /// The `+` chain around `[a, b)`: its token range.
    fn chain(&self, a: usize, b: usize) -> (usize, usize) {
        let boundary = |k: usize| {
            self.kw(k)
                || (self.toks[k].kind == Kind::Punct
                    && matches!(
                        self.src[self.toks[k].span.start as usize],
                        b',' | b';'
                            | b'='
                            | b':'
                            | b'!'
                            | b'&'
                            | b'|'
                            | b'<'
                            | b'>'
                            | b'-'
                            | b'*'
                            | b'/'
                            | b'%'
                            | b'^'
                    ))
        };
        let mut ra = a;
        while !self.starts(ra) {
            let t = ra - 1;
            if self.closer(t) {
                if self.mate[t] == NONE {
                    break;
                }
                ra = self.mate[t];
                continue;
            }
            if self.opener(t) || boundary(t) {
                break;
            }
            ra = t;
        }
        let mut rb = b;
        while rb < self.toks.len() {
            if self.nl[rb] && self.can_end(rb - 1) {
                break;
            }
            if self.closer(rb) || self.p(rb, b'{') || boundary(rb) {
                break;
            }
            if self.opener(rb) {
                if self.mate[rb] == NONE {
                    break;
                }
                rb = self.mate[rb] + 1;
                continue;
            }
            rb += 1;
        }
        (ra, rb)
    }

    /// The terms of the `+` chain `[a, b)`.
    fn terms(&self, a: usize, b: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = a;
        let mut k = a;
        while k < b {
            if self.opener(k) && self.mate[k] != NONE {
                k = self.mate[k] + 1;
                continue;
            }
            if self.p(k, b'+') {
                out.push((start, k));
                start = k + 1;
            }
            k += 1;
        }
        out.push((start, b));
        out
    }

    /// Follows the use of the located name at `i` in the function `f`.
    fn occurrence(&mut self, f: usize, i: usize) -> Step {
        let Some(name) = self.id(i) else {
            return Ok(());
        };
        let Some(vals) = self.vals[f].get(name).cloned() else {
            return Ok(());
        };
        // A field or a method of that name, a declaration, a target, a key.
        if i > 0 && (self.p(i - 1, b'.') || self.word(i - 1, b"var")) {
            return Ok(());
        }
        if self.target(i)? {
            return Ok(());
        }
        // `root string`: a parameter of a function literal, a field.
        if self.id(i + 1).is_some() && !self.nl[i + 1] {
            return Ok(());
        }
        if self.p(i + 1, b':')
            && i > 0
            && (self.p(i - 1, b'{') || self.p(i - 1, b','))
            && self.field_name(i)
        {
            return Ok(());
        }
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err(Coarse);
        }
        let (mut a, mut b) = (i, i + 1);
        let mut vals = vals;
        // The result of the expression is `(string, error)`.
        let mut pair = false;
        loop {
            // Parentheses that group.
            if a > 0 && self.p(a - 1, b'(') && self.mate[a - 1] == b && self.grouping(a - 1) {
                a -= 1;
                b += 1;
                continue;
            }
            // A whole argument of a path function.
            let whole = a > 0
                && (self.p(a - 1, b'(') || self.p(a - 1, b','))
                && (self.p(b, b')') || self.p(b, b','));
            if whole
                && let (Context::Call(open), index) = self.context(a)
                && let Some(Callee::Qualified(recv, fname, start)) = self.callee(open)
                && matches!(recv, b"filepath" | b"path")
            {
                if self.mate[open] == NONE {
                    return Err(Coarse);
                }
                let args = self.elements(open);
                let one = args.len() == 1;
                match fname {
                    b"Dir" if one => {
                        vals.iter_mut().for_each(Loc::up);
                        self.dirs.push(self.toks[open - 1].span.start);
                    }
                    b"Clean" | b"ToSlash" | b"FromSlash" | b"Abs" | b"EvalSymlinks" if one => {}
                    b"Join" if index == 0 => {
                        for &(x, y) in &args[1..] {
                            match self.literal(x, y) {
                                Some((s, at)) => {
                                    for v in &mut vals {
                                        s.split('/').for_each(|c| v.seg(c));
                                    }
                                    self.literals.push(at);
                                }
                                None => vals.iter_mut().for_each(Loc::unknown),
                            }
                            // Something below the file itself: not a path.
                            if vals.iter().any(|v| v.below_file) {
                                return Err(Coarse);
                            }
                        }
                    }
                    // A located path below another path.
                    b"Join" => return Err(Coarse),
                    _ => break,
                }
                pair = matches!(fname, b"Abs" | b"EvalSymlinks");
                a = start;
                b = self.mate[open] + 1;
                continue;
            }
            // A `+` chain.
            let plus = |k: usize| self.p(k, b'+') && !self.p(k + 1, b'=') && !self.p(k + 1, b'+');
            if plus(b) || (a > 0 && plus(a - 1) && !(a >= 2 && self.p(a - 2, b'+'))) {
                let (ra, rb) = self.chain(a, b);
                let terms = self.terms(ra, rb);
                let Some(own) = terms.iter().position(|&t| t == (a, b)) else {
                    return Err(Coarse);
                };
                if own != 0 {
                    // Text before the located path (`"--dir=" + root`): the path is
                    // handed on as it is.
                    return self.escape(a, &vals);
                }
                let mut sep = false;
                for &(x, y) in &terms[1..] {
                    match self.literal(x, y) {
                        Some((s, at)) => {
                            let mut after = sep;
                            for v in &mut vals {
                                after = v.text(&s, sep);
                            }
                            sep = after;
                            self.literals.push(at);
                        }
                        None => {
                            for v in &mut vals {
                                if sep {
                                    v.unknown();
                                } else {
                                    v.partial();
                                }
                            }
                            sep = false;
                        }
                    }
                }
                if vals.iter().any(|v| v.below_file) {
                    return Err(Coarse);
                }
                pair = false;
                (a, b) = (ra, rb);
                continue;
            }
            break;
        }
        if vals.iter().any(|v| v.ups > MAX_UPS) {
            return Err(Coarse);
        }
        self.consume(f, a, b, &vals, pair)
    }

    /// What becomes of the expression `[a, b)` with the values `vals`.
    fn consume(&mut self, f: usize, a: usize, b: usize, vals: &[Loc], pair: bool) -> Step {
        // A selector, an index, a slice, a call.
        if self.p(b, b'.') || self.p(b, b'[') || self.p(b, b'(') {
            return Err(Coarse);
        }
        // A comparison, on either side.
        let compared = |k: usize| {
            (self.p(k, b'=') && self.eq(k) == Eq::Compare) || self.p(k, b'<') || self.p(k, b'>')
        };
        if compared(b)
            || (self.p(b, b'!') && self.p(b + 1, b'=') && self.adjacent(b))
            || (a > 0 && compared(a - 1))
        {
            return Ok(());
        }
        // What follows.
        let ends = b >= self.toks.len()
            || (self.nl[b] && self.can_end(b - 1))
            || self.p(b, b',')
            || self.p(b, b';')
            || self.p(b, b':')
            || self.p(b, b'{')
            || self.closer(b);
        if !ends {
            return self.escape(a, vals);
        }
        // What comes before.
        if self.starts(a) {
            // An expression statement: the value is dropped.
            return Ok(());
        }
        let t = a - 1;
        let list = self.opener(t)
            || self.p(t, b',')
            || (self.p(t, b'=') && matches!(self.eq(t), Eq::Define | Eq::Assign))
            || self.kw(t);
        if !list {
            if self.p(t, b'&') {
                return Err(Coarse);
            }
            return self.escape(a, vals);
        }
        if self.word(t, b"switch") || self.word(t, b"if") {
            return Ok(());
        }
        let (context, index) = self.context(a);
        match context {
            Context::Call(open) => self.argument(f, open, index, a, vals),
            Context::Assign(eq, kind) => self.assign(f, eq, kind, index, a, b, vals, pair),
            Context::Case | Context::Decl => Ok(()),
            Context::Return | Context::Group | Context::Other => self.escape(a, vals),
        }
    }

    /// The values are the argument `index` of the call whose `(` is at `open`.
    fn argument(&mut self, f: usize, open: usize, index: usize, a: usize, vals: &[Loc]) -> Step {
        match self.callee(open) {
            Some(Callee::Bare(name)) => {
                if matches!(name, b"panic" | b"print" | b"println") {
                    return Ok(());
                }
                // A function of this file, unless a variable of that name hides it.
                if !self.funcs[f].declared.contains(name)
                    && let Some(&g) = self.by_name.get(name)
                    && let Some(&(Some(param), false)) = self.funcs[g].params.get(index)
                {
                    return self.add(g, param, vals);
                }
                self.escape(a, vals)
            }
            Some(Callee::Qualified(recv, name, _)) => {
                match recv {
                    b"filepath" | b"path" => {
                        return match name {
                            b"Rel" if index == 0 => Ok(()),
                            b"Base" | b"Ext" | b"IsAbs" | b"IsLocal" | b"VolumeName" => Ok(()),
                            b"Split" | b"SplitList" => Err(Coarse),
                            _ => self.escape(a, vals),
                        };
                    }
                    b"strings" | b"bytes" => {
                        return if measures(name) || (cuts(name) && index >= 1) {
                            Ok(())
                        } else {
                            // The path is cut or rewritten.
                            Err(Coarse)
                        };
                    }
                    b"fmt" if matches!(name, b"Sprintf" | b"Sprint" | b"Sprintln") => {
                        // Text can follow the path without a separator, unless the
                        // format says `%s/…`.
                        let first = self.elements(open).first().copied();
                        let joined = name == b"Sprintf"
                            && index == 1
                            && first
                                .and_then(|(x, y)| self.literal(x, y))
                                .is_some_and(|(s, _)| s.starts_with("%s/") || s.starts_with("%v/"));
                        if joined {
                            return self.escape(a, vals);
                        }
                        let mut up = vals.to_vec();
                        up.iter_mut().for_each(Loc::partial);
                        return self.escape(a, &up);
                    }
                    _ => {}
                }
                if crate::names::message_fn(name, Some(recv)) {
                    return Ok(());
                }
                self.escape(a, vals)
            }
            None => self.escape(a, vals),
        }
    }

    /// The values are the element `index` of the right side of the assignment
    /// at `eq`.
    #[allow(clippy::too_many_arguments)]
    fn assign(
        &mut self,
        f: usize,
        eq: usize,
        kind: Eq,
        index: usize,
        a: usize,
        b: usize,
        vals: &[Loc],
        pair: bool,
    ) -> Step {
        if kind == Eq::Op {
            return self.escape(a, vals);
        }
        let targets = self.targets(eq, kind);
        let count = index + 1 + self.elements_after(b);
        let (x, y) = if targets.len() == count {
            targets[index]
        } else if count == 1 && pair && targets.len() == 2 {
            targets[0]
        } else {
            return Err(Coarse);
        };
        let var = x > 0 && self.word(x - 1, b"var");
        // `var root string = …`
        let typed = var && y > x + 1 && self.id(x).is_some();
        if y == x + 1 || typed {
            let Some(name) = self.id(x) else {
                return self.escape(a, vals);
            };
            if name == b"_" {
                return Ok(());
            }
            if self.funcs[f].results.contains(&name) {
                // A named result: the value is returned.
                self.escape(a, vals)?;
            } else if !(kind == Eq::Define || var || self.funcs[f].declared.contains(name)) {
                // A package variable.
                return self.escape(a, vals);
            }
            return self.add(f, name, vals);
        }
        // `cmd.Dir = …`
        if y == x + 3
            && self.p(x + 1, b'.')
            && self.word(x + 2, b"Dir")
            && kind == Eq::Assign
            && let Some(cmd) = self.id(x)
        {
            return self.child(f, cmd, a, vals);
        }
        self.escape(a, vals)
    }

    /// The values are the working directory of the child process `cmd` of the
    /// function `f`.
    fn child(&mut self, f: usize, cmd: &'s [u8], a: usize, vals: &[Loc]) -> Step {
        let body = self.funcs[f].body;
        let mut words: Vec<String> = Vec::new();
        let mut commands = 0;
        for i in body.0 + 1..body.1 {
            if self.id(i) != Some(cmd) || self.p(i - 1, b'.') {
                continue;
            }
            // A field or a method other than a read of `Dir`.
            if self.p(i + 1, b'.') {
                let dir = self.word(i + 2, b"Dir");
                if !dir || (self.p(i + 3, b'=') && self.eq(i + 3) == Eq::Assign) {
                    continue;
                }
                return self.escape(a, vals);
            }
            // `cmd := exec.Command(…)`, `cmd = exec.CommandContext(ctx, …)`.
            let r = if self.p(i + 1, b':') && self.p(i + 2, b'=') {
                i + 3
            } else if self.p(i + 1, b'=') && self.eq(i + 1) == Eq::Assign {
                i + 2
            } else {
                return self.escape(a, vals);
            };
            let context = self.word(r + 2, b"CommandContext");
            if !(self.word(r, b"exec")
                && self.p(r + 1, b'.')
                && (context || self.word(r + 2, b"Command"))
                && self.p(r + 3, b'(')
                && self.mate[r + 3] != NONE
                && !self.p(self.mate[r + 3] + 1, b'.'))
            {
                return self.escape(a, vals);
            }
            commands += 1;
            for &(x, y) in self.elements(r + 3).iter().skip(usize::from(context)) {
                match self.literal(x, y) {
                    Some((s, _)) => words.push(s),
                    // A part of the command line that is not a literal.
                    None => return self.escape(a, vals),
                }
            }
        }
        if commands == 0 || vals.iter().any(|v| v.open || v.file) {
            return self.escape(a, vals);
        }
        // The paths of the command line are relative to the directory.
        for s in &words {
            for w in turborepo_trace_core::names::words(s) {
                if w.starts_with('/')
                    || !matches!(
                        turborepo_trace_core::names::shape_of(w, false),
                        Some((_, NameKind::Path | NameKind::File))
                    )
                {
                    continue;
                }
                for v in vals {
                    let mut v = v.clone();
                    w.split('/').for_each(|c| v.seg(c));
                    self.entry(a, v.rel())?;
                }
            }
        }
        Ok(())
    }

    /// The files `runtime.Caller` reports: `_, file, _, _ := runtime.Caller(0)`
    /// in a function binds `file`. `false`: there is none.
    fn seeds(&mut self) -> Step<bool> {
        let mut any = false;
        for i in 2..self.toks.len() {
            if !(self.word(i, b"Caller") && self.p(i - 1, b'.') && self.word(i - 2, b"runtime")) {
                continue;
            }
            let Some(f) = self.func_at(i) else {
                return Err(Coarse);
            };
            let zero = self
                .toks
                .get(i + 2)
                .is_some_and(|t| t.kind == Kind::Number && t.text(self.src) == b"0");
            if !(self.p(i + 1, b'(') && zero && self.p(i + 3, b')')) {
                // Another frame: the file of a caller.
                return Err(Coarse);
            }
            let after = i + 4;
            let ends = after >= self.toks.len()
                || (self.nl[after] && self.can_end(after - 1))
                || self.p(after, b';');
            if !(ends && i >= 3 && self.p(i - 3, b'=')) {
                return Err(Coarse);
            }
            let eq = i - 3;
            let kind = self.eq(eq);
            if !matches!(kind, Eq::Define | Eq::Assign) {
                return Err(Coarse);
            }
            let targets = self.targets(eq, kind);
            let name = |&(x, y): &(usize, usize)| self.id(x).filter(|_| y == x + 1);
            let names: Vec<Option<&'s [u8]>> = targets.iter().map(name).collect();
            // The program counter leads to the file as well.
            let pc = names.first().copied().flatten();
            let Some(file) = names.get(1).copied().flatten() else {
                return Err(Coarse);
            };
            if names.len() != 4 || pc != Some(b"_".as_slice()) {
                return Err(Coarse);
            }
            if file == b"_" {
                continue;
            }
            if kind == Eq::Assign && !self.funcs[f].declared.contains(file) {
                // A package variable.
                return Err(Coarse);
            }
            self.add(f, file, &[Loc::FILE])?;
            any = true;
        }
        Ok(any)
    }

    fn run(&mut self) -> Step {
        self.declarations();
        if !self.seeds()? {
            return Ok(());
        }
        for _ in 0..MAX_ROUNDS {
            self.changed = false;
            self.steps = 0;
            for f in 0..self.funcs.len() {
                if self.vals[f].is_empty() {
                    continue;
                }
                let (open, close) = self.funcs[f].body;
                for i in open + 1..close {
                    self.occurrence(f, i)?;
                }
            }
            if !self.changed {
                return Ok(());
            }
        }
        Err(Coarse)
    }
}

/// Follows the file `runtime.Caller` reports through the Go file `src`. `None`:
/// a use of it cannot be followed, and the file names everything below its
/// location.
pub fn follow(src: &[u8]) -> Option<Followed> {
    let mut w = Walk::new(src);
    w.run().ok()?;
    let line_of = |at: u32| {
        1 + memchr::memchr_iter(b'\n', &src[..(at as usize).min(src.len())]).count() as u32
    };
    let mut entries: Vec<Anchored> = w
        .entries
        .iter()
        .map(|(at, rel)| Anchored {
            cwd: false,
            line: line_of(*at),
            rel: rel.as_str().into(),
            pattern: EVERYTHING_BELOW.into(),
            conditional: false,
        })
        .collect();
    entries.sort_by_key(|e| e.line);
    w.literals.sort_unstable();
    w.literals.dedup();
    w.dirs.sort_unstable();
    w.dirs.dedup();
    Some(Followed {
        entries,
        literals: w.literals,
        dirs: w.dirs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entries of the body of a test function that has the file's directory
    /// two levels up in `root`: `(line, directory)`. Line 5 is the first
    /// line of `body`.
    fn of(body: &str, rest: &str) -> Option<Vec<(u32, String)>> {
        let src = format!(
            "package p\nfunc TestX(t *testing.T) {{\n\t_, file, _, _ := runtime.Caller(0)\n\troot \
             := filepath.Clean(filepath.Join(filepath.Dir(file), \"..\", \
             \"..\"))\n{body}\n}}\n{rest}"
        );
        entries(&src)
    }

    fn entries(src: &str) -> Option<Vec<(u32, String)>> {
        follow(src.as_bytes()).map(|f| {
            assert!(f.entries.iter().all(|e| !e.cwd && &*e.pattern == "**"));
            f.entries
                .into_iter()
                .map(|e| (e.line, e.rel.into_string()))
                .collect()
        })
    }

    fn some(entries: &[(u32, &str)]) -> Option<Vec<(u32, String)>> {
        Some(entries.iter().map(|&(l, r)| (l, r.to_owned())).collect())
    }

    #[test]
    fn a_root_joined_with_literal_segments_names_that_subtree() {
        // A known path and a path with an unknown tail, read in the same function.
        assert_eq!(
            of(
                "\tos.ReadFile(filepath.Join(root, \"configs\", \
                 \"app.json\"))\n\tos.ReadDir(filepath.Join(root, \"services\", name))",
                ""
            ),
            some(&[(5, "../../configs/app.json"), (6, "../../services")])
        );
        // Through a variable, with a segment written as a path, and by `+`.
        assert_eq!(
            of(
                "\tdir := filepath.Join(root, \
                 \"services/svc-a\")\n\tother.Load(dir)\n\tother.Load(root + \"/configs/\" + \
                 name)\n\tother.Load(root + \"/data/x\" + suffix)",
                ""
            ),
            some(&[
                (6, "../../services/svc-a"),
                (7, "../../configs"),
                (8, "../../data")
            ])
        );
        // Text appended without a separator is a sibling.
        assert_eq!(
            of("\tother.Load(root + \"-old\")", ""),
            some(&[(5, "../../..")])
        );
        // A parent step after an unknown part raises what is known.
        assert_eq!(
            of(
                "\tother.Load(filepath.Join(root, \"a\", \"b\", name, \"..\"))",
                ""
            ),
            some(&[(5, "../../a")])
        );
        // The directory of the file, by `Dir` and by a parent step.
        assert_eq!(
            entries(
                "package p\nfunc f() {\n\t_, file, _, _ := runtime.Caller(0)\n\tt.Logf(\"in %s\", \
                 file)\n\tother.Use(filepath.Dir(file))\n\tother.Use(filepath.Join(file, \"..\", \
                 \"testdata\"))\n}\n"
            ),
            some(&[(5, ""), (6, "testdata")])
        );
    }

    #[test]
    fn what_is_no_read_names_nothing() {
        let body = "\trel, _ := filepath.Rel(root, other)\n\tt.Logf(\"root %s %s\", root, \
                    rel)\n\tif root == other || other != root || strings.HasPrefix(other, root) \
                    {\n\t\tpanic(root)\n\t}\n\tswitch root {\n\tcase other, root:\n\t}\n\t_ = \
                    filepath.Base(root)\n\t_ = fmt.Errorf(\"in %s\", root)\n\tabs, err := \
                    filepath.Abs(root)\n\t_, _ = abs, err";
        assert_eq!(of(body, ""), some(&[]));
        // The target of `Rel` is a path that is handed on.
        assert_eq!(
            of("\trel, _ := filepath.Rel(other, root)\n\t_ = rel", ""),
            some(&[(5, "../..")])
        );
    }

    #[test]
    fn the_working_directory_of_a_child_process() {
        // A literal command line: nothing is read by this process.
        assert_eq!(
            of(
                "\tcmd := exec.Command(\"git\", \"rev-parse\", \"HEAD\")\n\tcmd.Dir = \
                 root\n\tcmd.Env = os.Environ()\n\tout, _ := cmd.Output()\n\t_ = out",
                ""
            ),
            some(&[])
        );
        // Its words that are paths are relative to the directory.
        assert_eq!(
            of(
                "\tcmd := exec.CommandContext(ctx, \"go\", \"run\", \"./cmd/gen\", \
                 \"--config=configs/a.json\", \"-v\")\n\tcmd.Dir = filepath.Join(root, \
                 \"tools\")\n\tcmd.Run()",
                ""
            ),
            some(&[
                (6, "../../tools/cmd/gen"),
                (6, "../../tools/configs/a.json")
            ])
        );
        // A part that is not a literal: the directory escapes.
        assert_eq!(
            of(
                "\tcmd := exec.Command(\"pnpm\", \"run\", script)\n\tcmd.Dir = root\n\tcmd.Run()",
                ""
            ),
            some(&[(6, "../..")])
        );
        // The command is handed on, its directory is read back, or it is no command.
        for body in [
            "\tcmd := exec.Command(\"make\")\n\tcmd.Dir = root\n\tstart(cmd)",
            "\tcmd := exec.Command(\"make\")\n\tcmd.Dir = root\n\tos.ReadFile(cmd.Dir)",
            "\tcmd := newThing()\n\tcmd.Dir = root",
        ] {
            assert_eq!(of(body, ""), some(&[(6, "../..")]), "{body}");
        }
        assert_eq!(of("\th.Dir = root", ""), some(&[(5, "../..")]));
    }

    #[test]
    fn a_function_of_the_file_gets_the_value_as_its_parameter() {
        let rest = "func sweep(ctx context.Context, dir string) {\n\tcmd := \
                    exec.CommandContext(ctx, \"docker\", \"ps\")\n\tcmd.Dir = \
                    dir\n\tcmd.Run()\n}\nfunc load(t *testing.T, base, name string) []byte \
                    {\n\tb, _ := os.ReadFile(filepath.Join(base, \"testdata\", name))\n\treturn \
                    b\n}\nfunc run(dir string, name string, args ...string) error {\n\tcmd := \
                    exec.Command(name, args...)\n\tcmd.Dir = dir\n\treturn cmd.Run()\n}\n";
        assert_eq!(of("\tsweep(ctx, root)", rest), some(&[]));
        assert_eq!(
            of("\tload(t, filepath.Join(root, \"pkg\"), \"a.json\")", rest),
            some(&[(13, "../../pkg/testdata")])
        );
        // The helper runs a command it is given: its parts are not literals there.
        assert_eq!(
            of("\trun(root, \"git\", \"status\")", rest),
            some(&[(18, "../..")])
        );
        // A variadic or a missing parameter, a method, a function of another file, and
        // a variable that hides the function.
        for body in [
            "\trun(\"x\", \"cat\", root)",
            "\th.sweep(ctx, root)",
            "\tsweepElsewhere(ctx, root)",
            "\tsweep := pick()\n\tsweep(ctx, root)",
        ] {
            let line = if body.starts_with("\tsweep :=") { 6 } else { 5 };
            assert_eq!(of(body, rest), some(&[(line, "../..")]), "{body}");
        }
    }

    #[test]
    fn what_reaches_package_scope_escapes() {
        // The shapes of a harness: a function that returns the root, with and without
        // resolving links.
        let plain = "package e2e\nfunc repoRoot() string {\n\t_, file, _, _ := \
                     runtime.Caller(0)\n\treturn filepath.Clean(filepath.Join(filepath.Dir(file), \
                     \"..\", \"..\", \"..\"))\n}\n";
        assert_eq!(entries(plain), some(&[(4, "../../..")]));
        let links = "package e2e\nfunc repoRoot() string {\n\t_, file, _, _ := \
                     runtime.Caller(0)\n\troot := \
                     filepath.Clean(filepath.Join(filepath.Dir(file), \"..\", \"..\", \
                     \"..\"))\n\tif real, err := filepath.EvalSymlinks(root); err == nil \
                     {\n\t\treturn real\n\t}\n\treturn root\n}\n";
        assert_eq!(entries(links), some(&[(6, "../../..")]));
        // A subdirectory is all that is returned.
        let services = "package d\nfunc servicesDir(t *testing.T) string {\n\t_, thisFile, _, ok \
                        := runtime.Caller(0)\n\tif !ok {\n\t\tt.Fatalf(\"no \
                        caller\")\n\t}\n\treturn filepath.Join(filepath.Dir(thisFile), \"..\", \
                        \"..\", \"..\", \"..\", \"services\")\n}\n";
        assert_eq!(entries(services), some(&[(7, "../../../../services")]));
        for (body, line) in [
            // A struct field, a composite literal, a map, a slice, a channel.
            ("\th := &harness{repoRoot: root, n: 1}\n\t_ = h", 5),
            ("\th.repoRoot = root", 5),
            ("\tdirs := []string{root}\n\t_ = dirs", 5),
            ("\tm[\"root\"] = root", 5),
            ("\tm := map[string]int{root: 1}\n\t_ = m", 5),
            ("\th := &harness{root: root}\n\t_ = h", 5),
            ("\tdirs = append(dirs, root)", 5),
            ("\tch <- root", 5),
            // A package variable, the environment, another package, a closure's result.
            ("\tshared = root", 5),
            ("\tos.Setenv(\"ROOT\", root)", 5),
            ("\tos.Chdir(root)", 5),
            ("\tgo watch(\"--dir=\" + root)", 5),
            ("\tf := func() string {\n\t\treturn root\n\t}\n\t_ = f", 6),
        ] {
            assert_eq!(of(body, ""), some(&[(line, "../..")]), "{body}");
        }
        // A named result.
        let named = "package p\nfunc root() (dir string, err error) {\n\t_, file, _, _ := \
                     runtime.Caller(0)\n\tdir = filepath.Dir(file)\n\treturn\n}\n";
        assert_eq!(entries(named), some(&[(4, "")]));
        // Text of a format: a separator after the path keeps it, anything else may be a
        // sibling.
        assert_eq!(
            of(
                "\tp := fmt.Sprintf(\"%s/out/%d\", root, n)\n\tother.Load(p)",
                ""
            ),
            some(&[(5, "../..")])
        );
        assert_eq!(
            of(
                "\tp := fmt.Sprintf(\"%s-%d\", root, n)\n\tother.Load(p)",
                ""
            ),
            some(&[(5, "../../..")])
        );
    }

    #[test]
    fn a_use_that_cannot_be_followed_keeps_the_coarse_mark() {
        for body in [
            "\tother.Load(root[1:])",
            "\tother.Load(&root)",
            "\troot += \"/x\"",
            "\tother.Load(strings.TrimSuffix(root, \"/e2e\"))",
            "\tother.Load(strings.ReplaceAll(root, \"a\", \"b\"))",
            "\tother.Load(filepath.Join(base, root))",
            "\tdir, _ := filepath.Split(root)\n\t_ = dir",
            "\ta, b := root, 1, 2",
        ] {
            assert_eq!(of(body, ""), None, "{body}");
        }
        for src in [
            // The file of a caller, a kept program counter, a call that is not bound.
            "package p\nfunc f() string {\n\t_, file, _, _ := runtime.Caller(1)\n\treturn \
             filepath.Dir(file)\n}\n",
            "package p\nfunc f() string {\n\tpc, file, _, _ := runtime.Caller(0)\n\t_ = \
             pc\n\treturn filepath.Dir(file)\n}\n",
            "package p\nfunc f() {\n\tg(runtime.Caller(0))\n}\n",
            // The file itself is handed on: the parent step of whoever gets it only removes
            // the file name.
            "package p\nfunc thisFile() string {\n\t_, file, _, _ := runtime.Caller(0)\n\treturn \
             file\n}\n",
            "package p\nfunc f() {\n\t_, file, _, _ := runtime.Caller(0)\n\tother.Use(file)\n}\n",
            "package p\nfunc f() {\n\t_, file, _, _ := runtime.Caller(0)\n\tother.Use(file + \
             \".golden\")\n}\n",
            "package p\nfunc f() {\n\t_, file, _, _ := \
             runtime.Caller(0)\n\tother.Use(filepath.Join(file, name))\n}\n",
            "package p\nfunc f() {\n\t_, file, _, _ := \
             runtime.Caller(0)\n\tother.Use(fmt.Sprintf(\"%s.bak\", file))\n}\n",
            // Outside a function declaration, and a package variable.
            "package p\nvar root = func() string {\n\t_, file, _, _ := \
             runtime.Caller(0)\n\treturn filepath.Dir(file)\n}()\n",
            "package p\nvar file string\nfunc init() {\n\t_, file, _, _ = runtime.Caller(0)\n}\n",
        ] {
            assert_eq!(entries(src), None, "{src}");
        }
        // No file is bound: there is no location.
        assert_eq!(
            entries(
                "package p\nfunc f() int {\n\t_, _, line, _ := runtime.Caller(0)\n\treturn \
                 line\n}\n"
            ),
            some(&[])
        );
    }

    #[test]
    fn names_of_a_followed_file() {
        use crate::names::{names, names_with};
        let src = "package d\nfunc servicesDir(t *testing.T) string {\n\t_, thisFile, _, ok := \
                   runtime.Caller(0)\n\tif !ok {\n\t\tt.Fatalf(\"no caller\")\n\t}\n\treturn \
                   filepath.Join(filepath.Dir(thisFile), \"..\", \"..\", \"..\", \"..\", \
                   \"services\")\n}\n";
        // Without following: everything below the location, raised by five parent
        // steps.
        let coarse = names_with(src.as_bytes(), false, false);
        assert_eq!((coarse.anchor, coarse.ups), (Some(7), 5));
        assert!(coarse.anchored.is_empty());
        assert!(coarse.names[0].climbs);
        // With: the directory, and no parent step is left to apply to it.
        let n = names(src.as_bytes(), false);
        assert_eq!((n.anchor, n.ups), (None, 0));
        assert_eq!(
            n.encode(),
            "names:0,0,0,0,0\n7\tservices\n@L\t7\t../../../../services\t**"
        );
        assert!(!n.names[0].climbs);
        // Parent steps elsewhere in the file still count, for every entry.
        let more = format!(
            "{src}func other(p string) string {{\n\treturn filepath.Join(filepath.Dir(p), \"..\", \
             \"x\")\n}}\n"
        );
        let n = names(more.as_bytes(), false);
        assert_eq!((n.anchor, n.ups, n.anchored.len()), (None, 2, 1));
        // A file whose location cannot be followed is as before.
        let cut = "package p\nfunc f() string {\n\t_, file, _, _ := runtime.Caller(0)\n\treturn \
                   strings.TrimSuffix(filepath.Dir(file), \"/e2e\")\n}\n";
        assert_eq!(
            names(cut.as_bytes(), false),
            names_with(cut.as_bytes(), false, false)
        );
        assert_eq!(names(cut.as_bytes(), false).anchor, Some(4));
    }

    #[test]
    fn malformed_input_does_not_panic() {
        for src in [
            "package p\nfunc f() {\n\t_, file, _, _ := runtime.Caller(0)\n\tx := \
             filepath.Join(file",
            "runtime.Caller(0)",
            "= runtime.Caller(0)",
            "func f() { _, file, _, _ := runtime.Caller(0); file",
            "func f() { _, file, _, _ := runtime.Caller(0); (file) + }",
            "func f( { _, file, _, _ := runtime.Caller(0) }",
            "func (f) { , file := runtime.Caller(0) }",
            "func f() { _, file, _, _ := runtime.Caller(0)\n= file }",
            "func f() { _, file, _, _ := runtime.Caller(0)\nfile = }",
            "func f() { _, file, _, _ := runtime.Caller(0)\nx := filepath.Join(file, \"a\"] }",
            "func f() { _, file, _, _ := runtime.Caller(0)\nx := filepath.Dir(file] }",
            "func f() { _, file, _, _ := runtime.Caller(0)\ng(file, 1] }\nfunc g(a string",
            "func f() { _, file, _, _ := runtime.Caller(0)\nc := exec.Command(\"a\"]\nc.Dir = \
             file }",
            "func f() { _, file, _, _ := runtime.Caller(0)\nx := []string{file: 1 }",
            "func f() { _, file, _, _ := runtime.Caller(0)\nx := file +",
            "func f() { _, file, _, _ := runtime.Caller(0)\n+ file",
            "func f() { _, file, _, _ := runtime.Caller(0)\nx := filepath.Join(file, \"a\"]) }",
            "func f() { _, file, _, _ := runtime.Caller(0)\ng(file}, 1) }\nfunc g(a string) {}",
        ] {
            let _ = follow(src.as_bytes());
        }
        // Every prefix of a file with most of the constructs, and the same with one
        // bracket removed.
        let src = "package p\nfunc TestX(t *testing.T) {\n\t_, file, _, _ := \
                   runtime.Caller(0)\n\troot := filepath.Clean(filepath.Join(filepath.Dir(file), \
                   \"..\", \"..\"))\n\tif real, err := filepath.EvalSymlinks(root); err == nil \
                   {\n\t\troot = real\n\t}\n\tcmd := exec.Command(\"go\", \"run\", \
                   \"./cmd/gen\")\n\tcmd.Dir = root\n\th := &harness{root: root, dirs: \
                   []string{root + \"/a/\" + name}}\n\tload(t, (root), fmt.Sprintf(\"%s/x\", \
                   root))\n\tif root == other || strings.HasPrefix(other, root) \
                   {\n\t\tpanic(root)\n\t}\n\tswitch root {\n\tcase other, \
                   root:\n\t}\n\treturn\n}\nfunc load(t *testing.T, base, name string) (out \
                   []byte, err error) {\n\tout, err = os.ReadFile(filepath.Join(base, \
                   \"testdata\", name))\n\treturn\n}\n";
        assert!(follow(src.as_bytes()).is_some());
        for cut in 0..src.len() {
            let _ = follow(&src.as_bytes()[..cut]);
            if matches!(src.as_bytes()[cut], b'(' | b')' | b'{' | b'}' | b'[' | b']') {
                let mut without = src.as_bytes().to_vec();
                without.remove(cut);
                let _ = follow(&without);
            }
        }
    }
}
