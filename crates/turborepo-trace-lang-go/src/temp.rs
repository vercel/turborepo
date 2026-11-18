//! Names that hold a path below a temporary directory.
//!
//! A Go test typically writes its inputs below `t.TempDir()` and reads them
//! back:
//!
//! ```go
//! dir := t.TempDir()
//! path := filepath.Join(dir, "config.json")
//! os.WriteFile(path, data, 0o600)
//! got, err := os.ReadFile(path)
//! ```
//!
//! Nothing below a temporary directory is a repository file, so such a read
//! depends on no repository file, and `"config.json"` there names none.
//! [`TempRoots`] finds the names for which that is certain, per top-level
//! function, without tracking control flow:
//!
//! > a name is a temp root in a function when **every** binding of the name in
//! > that
//! > function binds a value below a temporary directory, and the name is
//! > nothing else there.
//!
//! Values below a temporary directory ([`TempRoots::is_temp_value`]):
//!
//! - `t.TempDir()` on a `testing` handle (a name declared as `*testing.T`,
//!   `*testing.B`, `*testing.F` or `testing.TB` in a signature of the file),
//!   `os.TempDir()`;
//! - `os.MkdirTemp(dir, pattern)` / `ioutil.TempDir(dir, pattern)` with `dir`
//!   the empty string or a temp root (the first of the two results);
//! - a temp root name; `filepath.Join(<temp>, …)` / `path.Join(<temp>, …)`;
//!   `<temp> + …`.
//!
//! with no parent step in the value: no `..` segment in a string, no
//! `filepath.Dir` call (the parent of a temporary directory is not one).
//!
//! The name must be declared in the function (`name := …` or `var name = …`): a
//! name that is only assigned there is a package-level variable. A name is
//! "something else" when the function (nested function literals included)
//! declares it in a signature, declares it with `var` without a value, takes
//! its address, assigns it with a compound operator, binds it at any other
//! position of a tuple, or binds it to a value too long to read. Bindings
//! outside functions never count: a package-level variable can be assigned from
//! another file.
//!
//! Sound for locals because the rule quantifies over every binding in the
//! function: whichever one reaches a use, the value is below a temporary
//! directory. Not modelled: a name that shadows a package-level variable of
//! another file in one block and is used in another block of the same function
//! (Go block scoping), and `TMPDIR` pointing into the repository.

use std::collections::HashSet;

use crate::lex::{Kind, Tok, string_value};

/// Values longer than this many tokens are not read.
const MAX_VALUE_TOKENS: usize = 256;

/// The temp roots of a file (see the module docs).
#[derive(Debug, Default)]
pub struct TempRoots {
    /// Per token of the file: the top-level function it is in (1-based), 0
    /// outside.
    func_of: Vec<u32>,
    roots: HashSet<(u32, Vec<u8>)>,
    handles: HashSet<Vec<u8>>,
}

/// One `name := value` / `name = value`.
struct Bind<'s> {
    func: u32,
    name: &'s [u8],
    /// Significant-token range of the value.
    value: (usize, usize),
    /// `name := value` or `var name = value`: the name is declared here, so it
    /// is a local.
    declares: bool,
}

impl TempRoots {
    /// Whether the file can have temp roots at all (a cheap test before
    /// [`TempRoots::of`]).
    pub fn possible(src: &[u8]) -> bool {
        memchr::memmem::find(src, b"TempDir").is_some()
            || memchr::memmem::find(src, b"MkdirTemp").is_some()
    }

    /// The temp roots of the file with tokens `toks`.
    pub fn of(src: &[u8], toks: &[Tok]) -> TempRoots {
        let sig: Vec<usize> = (0..toks.len()).filter(|&i| !toks[i].is_comment()).collect();
        let tok = |k: usize| toks[sig[k]];
        let text = |k: usize| tok(k).text(src);
        let is = |k: usize, b: u8| k < sig.len() && tok(k).is_punct(src, b);
        let ident = |k: usize| k < sig.len() && tok(k).kind == Kind::Ident;
        let adjacent = |a: usize, b: usize| tok(a).span.end == tok(b).span.start;

        // Pass 1: the function of every token.
        let mut func_of_sig = vec![0u32; sig.len()];
        let (mut depth, mut cur, mut funcs) = (0i32, 0u32, 0u32);
        for (k, slot) in func_of_sig.iter_mut().enumerate() {
            if depth == 0 && tok(k).is_ident(src, b"func") {
                funcs += 1;
                cur = funcs;
            }
            *slot = cur;
            if is(k, b'{') {
                depth += 1;
            } else if is(k, b'}') {
                depth -= 1;
                if depth <= 0 {
                    depth = 0;
                    cur = 0;
                }
            }
        }
        let mut func_of = vec![0u32; toks.len()];
        for (k, &i) in sig.iter().enumerate() {
            func_of[i] = func_of_sig[k];
        }

        // Pass 2: bindings, and the names that can hold something else.
        let mut other: HashSet<(u32, &[u8])> = HashSet::new();
        let mut handles: HashSet<Vec<u8>> = HashSet::new();
        let mut binds: Vec<Bind<'_>> = Vec::new();
        for (k, &f) in func_of_sig.iter().enumerate() {
            let t = tok(k);
            // A signature: every identifier inside its parentheses is declared there.
            if t.is_ident(src, b"func") {
                let mut j = k + 1;
                let mut par = 0usize;
                while j < sig.len() {
                    if is(j, b'(') || is(j, b'[') {
                        par += 1;
                    } else if is(j, b')') || is(j, b']') {
                        par = par.saturating_sub(1);
                    } else if is(j, b'{') {
                        // `interface{…}` / `struct{…}` inside the signature.
                        let ty = tok(j - 1).is_ident(src, b"interface")
                            || tok(j - 1).is_ident(src, b"struct");
                        if !ty && par == 0 {
                            break;
                        }
                    } else if ident(j) && par > 0 {
                        other.insert((f, text(j)));
                        // `name, other *testing.T`: testing handles.
                        if text(j) == b"testing"
                            && is(j + 1, b'.')
                            && ident(j + 2)
                            && matches!(text(j + 2), b"T" | b"B" | b"F" | b"TB")
                        {
                            let mut h = j;
                            if h > 0 && is(h - 1, b'*') {
                                h -= 1;
                            }
                            while h > 0 && ident(h - 1) {
                                handles.insert(text(h - 1).to_vec());
                                h -= 1;
                                if h > 0 && is(h - 1, b',') {
                                    h -= 1;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else if par == 0
                        && j > k + 1
                        && ident(j)
                        && matches!(text(j), b"func" | b"var" | b"const" | b"type" | b"import")
                    {
                        // A declaration without a body.
                        break;
                    }
                    j += 1;
                }
                continue;
            }
            // `var name Type`, `var name Type = …`: the name can hold anything. `var name =
            // value` is a binding like `name := value`.
            if t.is_ident(src, b"var") && ident(k + 1) && !is(k + 2, b'=') {
                let mut j = k + 1;
                while ident(j) {
                    other.insert((f, text(j)));
                    if is(j + 1, b',') {
                        j += 2;
                    } else {
                        break;
                    }
                }
            }
            // `&name` (not the binary operator).
            if is(k, b'&')
                && ident(k + 1)
                && !(k > 0 && (ident(k - 1) || is(k - 1, b')') || is(k - 1, b']')))
            {
                other.insert((f, text(k + 1)));
            }
            if !is(k, b'=') || k == 0 {
                continue;
            }
            // `==`, `!=`, `<=`, `>=` are comparisons; `<<=` and `>>=` assign.
            let shift = k >= 2
                && adjacent(k - 2, k - 1)
                && adjacent(k - 1, k)
                && ((is(k - 2, b'<') && is(k - 1, b'<')) || (is(k - 2, b'>') && is(k - 1, b'>')));
            if (is(k + 1, b'=') && adjacent(k, k + 1))
                || (adjacent(k - 1, k)
                    && !shift
                    && (is(k - 1, b'=') || is(k - 1, b'!') || is(k - 1, b'<') || is(k - 1, b'>')))
            {
                continue;
            }
            let short = is(k - 1, b':') && adjacent(k - 1, k);
            // `+=`, `<<=`, ...: the name changes.
            let compound = !short && tok(k - 1).kind == Kind::Punct && adjacent(k - 1, k);
            let mut j = if short { k - 1 } else { k };
            if compound {
                while j > 0 && tok(j - 1).kind == Kind::Punct && adjacent(j - 1, j) {
                    j -= 1;
                }
            }
            // Targets: identifiers separated by commas.
            let mut targets: Vec<&[u8]> = Vec::new();
            while j > 0 && ident(j - 1) {
                targets.push(text(j - 1));
                j -= 1;
                if j > 0 && is(j - 1, b',') {
                    j -= 1;
                } else {
                    break;
                }
            }
            // `x.name = …` assigns a field.
            if j > 0 && is(j - 1, b'.') {
                targets.pop();
            }
            targets.reverse();
            if targets.is_empty() {
                continue;
            }
            let declares = short || (j > 0 && tok(j - 1).is_ident(src, b"var"));
            // The value: to the end of the statement.
            let start = k + 1;
            let mut end = start;
            let mut nest = 0i32;
            while end < sig.len() && end - start < MAX_VALUE_TOKENS {
                let u = tok(end);
                if end > start && nest == 0 {
                    let p = tok(end - 1);
                    let continues = p.kind == Kind::Punct
                        && matches!(src[p.span.start as usize], b'+' | b',' | b'.' | b'(');
                    let gap = &src[p.span.end as usize..u.span.start as usize];
                    if memchr::memchr(b'\n', gap).is_some() && !continues {
                        break;
                    }
                }
                if u.kind == Kind::Punct {
                    match src[u.span.start as usize] {
                        b'(' | b'[' | b'{' => nest += 1,
                        b')' | b']' | b'}' => {
                            nest -= 1;
                            if nest < 0 {
                                break;
                            }
                        }
                        b';' if nest == 0 => break,
                        _ => {}
                    }
                }
                end += 1;
            }
            let unreadable = compound || end - start >= MAX_VALUE_TOKENS;
            for (i, name) in targets.iter().enumerate() {
                if *name == b"_" {
                    continue;
                }
                // Only the first target of `dir, err := os.MkdirTemp(…)` is the directory.
                if unreadable || i > 0 {
                    other.insert((f, name));
                } else {
                    binds.push(Bind {
                        func: f,
                        name,
                        value: (start, end),
                        declares,
                    });
                }
            }
        }

        // Fixpoint: a name is a temp root when every binding of it in its function is a
        // temp value.
        let mut out = TempRoots {
            func_of,
            roots: HashSet::new(),
            handles,
        };
        // Declared in the function (a local), and nothing else there.
        let mut candidates: Vec<(u32, &[u8])> = binds
            .iter()
            .filter(|b| b.func != 0 && b.declares && !other.contains(&(b.func, b.name)))
            .map(|b| (b.func, b.name))
            .collect();
        candidates.sort_unstable();
        candidates.dedup();
        loop {
            let mut grew = false;
            for &(f, name) in &candidates {
                if out.roots.contains(&(f, name.to_vec())) {
                    continue;
                }
                let own = || binds.iter().filter(|b| b.func == f && b.name == name);
                // One binding must be a temp value on its own; the others may build on the
                // name itself (`home = filepath.Join(home, "user")`).
                let base = own().any(|b| out.temp_value(src, toks, &sig, b.value.0, b.value.1, f));
                if !base {
                    continue;
                }
                out.roots.insert((f, name.to_vec()));
                if own().all(|b| out.temp_value(src, toks, &sig, b.value.0, b.value.1, f)) {
                    grew = true;
                } else {
                    out.roots.remove(&(f, name.to_vec()));
                }
            }
            if !grew {
                break;
            }
        }
        out
    }

    /// Whether `name`, used at token `at` (an index into the file's tokens), is
    /// a temp root there.
    pub fn contains(&self, at: usize, name: &[u8]) -> bool {
        let f = self.func_of.get(at).copied().unwrap_or(0);
        f != 0 && self.roots.contains(&(f, name.to_vec()))
    }

    /// Whether `name` is declared as a `testing` handle in a signature of the
    /// file.
    pub fn is_handle(&self, name: &[u8]) -> bool {
        self.handles.contains(name)
    }

    /// Whether the tokens `toks[a..b]` of the file are a value below a
    /// temporary directory, as seen from the function of token `a`.
    /// Comments in the range are skipped.
    pub fn is_temp_value(&self, src: &[u8], toks: &[Tok], a: usize, b: usize) -> bool {
        let sig: Vec<usize> = (a..b.min(toks.len()))
            .filter(|&i| !toks[i].is_comment())
            .collect();
        let f = self.func_of.get(a).copied().unwrap_or(0);
        self.temp_value(src, toks, &sig, 0, sig.len(), f)
    }

    fn root(&self, f: u32, name: &[u8]) -> bool {
        f != 0 && self.roots.contains(&(f, name.to_vec()))
    }

    /// [`TempRoots::is_temp_value`] over the significant tokens `sig[a..b]` in
    /// function `f`.
    fn temp_value(
        &self,
        src: &[u8],
        toks: &[Tok],
        sig: &[usize],
        a: usize,
        b: usize,
        f: u32,
    ) -> bool {
        let b = b.min(sig.len());
        if a >= b {
            return false;
        }
        let tok = |k: usize| toks[sig[k]];
        let is = |k: usize, c: u8| k < b && tok(k).is_punct(src, c);
        let ident = |k: usize| k < b && tok(k).kind == Kind::Ident;
        // No parent step anywhere in the value.
        for k in a..b {
            let t = tok(k);
            if t.is_string()
                && string_value(src, &t).is_none_or(|v| v.split(['/', '\\']).any(|s| s == ".."))
            {
                return false;
            }
            if t.is_ident(src, b"Dir") && is(k + 1, b'(') {
                return false;
            }
        }
        if !ident(a) {
            return false;
        }
        let first = tok(a).text(src);
        // `name`, `name + …`.
        if self.root(f, first) && (b == a + 1 || is(a + 1, b'+')) {
            return true;
        }
        // `recv.Func(…)` spanning the whole value.
        if !(is(a + 1, b'.') && ident(a + 2) && is(a + 3, b'(')) {
            return false;
        }
        let mut nest = 0i32;
        let mut close = None;
        for k in a + 3..b {
            if is(k, b'(') {
                nest += 1;
            } else if is(k, b')') {
                nest -= 1;
                if nest == 0 {
                    close = Some(k);
                    break;
                }
            }
        }
        if close != Some(b - 1) {
            return false;
        }
        let arg = a + 4;
        match tok(a + 2).text(src) {
            // `t.TempDir()`, `os.TempDir()`.
            b"TempDir" if arg == b - 1 => first == b"os" || self.is_handle(first),
            // `os.MkdirTemp(dir, pattern)`, `ioutil.TempDir(dir, pattern)`.
            b"MkdirTemp" | b"TempDir" if matches!(first, b"os" | b"ioutil") => {
                let d = tok(arg);
                (d.is_string() && string_value(src, &d).is_some_and(|v| v.is_empty()))
                    || (ident(arg) && self.root(f, d.text(src)) && is(arg + 1, b','))
            }
            // `filepath.Join(<temp>, …)`.
            b"Join" if matches!(first, b"filepath" | b"path") => {
                self.temp_first_arg(src, toks, sig, arg, b, f)
            }
            _ => false,
        }
    }

    /// Whether the first argument starting at `sig[arg]` is a temp root name or
    /// a direct `t.TempDir()` / `os.TempDir()` call.
    fn temp_first_arg(
        &self,
        src: &[u8],
        toks: &[Tok],
        sig: &[usize],
        arg: usize,
        b: usize,
        f: u32,
    ) -> bool {
        let tok = |k: usize| toks[sig[k]];
        let is = |k: usize, c: u8| k < b && tok(k).is_punct(src, c);
        if arg >= b || tok(arg).kind != Kind::Ident {
            return false;
        }
        let name = tok(arg).text(src);
        if self.root(f, name) && (is(arg + 1, b',') || is(arg + 1, b')')) {
            return true;
        }
        is(arg + 1, b'.')
            && arg + 2 < b
            && tok(arg + 2).is_ident(src, b"TempDir")
            && is(arg + 3, b'(')
            && is(arg + 4, b')')
            && (is(arg + 5, b',') || is(arg + 5, b')'))
            && (name == b"os" || self.is_handle(name))
    }

    /// Whether the call whose `(` is token `open` has a first argument below a
    /// temporary directory: a temp root name or a direct `TempDir()` call.
    pub fn call_below_temp(&self, src: &[u8], toks: &[Tok], open: usize) -> bool {
        // The significant tokens of a short window after the parenthesis.
        let sig: Vec<usize> = (open + 1..toks.len().min(open + 16))
            .filter(|&i| !toks[i].is_comment())
            .collect();
        let f = self.func_of.get(open).copied().unwrap_or(0);
        self.temp_first_arg(src, toks, &sig, 0, sig.len(), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::tokenize;

    /// The temp roots as `function:name`, sorted.
    fn roots(src: &str) -> Vec<String> {
        let (toks, _) = tokenize(src.as_bytes());
        let r = TempRoots::of(src.as_bytes(), &toks);
        let mut v: Vec<String> = r
            .roots
            .iter()
            .map(|(f, n)| format!("{f}:{}", String::from_utf8_lossy(n)))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn every_binding_must_be_below_a_temp_dir() {
        let src = r#"package p
func TestA(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	work, err := os.MkdirTemp("", "x")
	nested, err := os.MkdirTemp(dir, "y")
	home := t.TempDir()
	home = filepath.Join(home, "user")
	mixed := t.TempDir()
	if cond {
		mixed = "testdata"
	}
	up := filepath.Join(dir, "..", "x")
	parent := filepath.Dir(dir)
	plus := dir + "/a"
	sys := os.TempDir()
}
func TestB(t *testing.T) {
	dir := "testdata"
	plus := filepath.Join(dir, "x")
}
"#;
        assert_eq!(
            roots(src),
            [
                "1:dir", "1:home", "1:nested", "1:plus", "1:sub", "1:sys", "1:work"
            ]
        );
    }

    #[test]
    fn names_that_can_hold_something_else() {
        let src = r#"package p
var shared = os.TempDir()
func helper(t *testing.T, dir string) {
	dir = t.TempDir()
}
func TestA(t *testing.T) {
	for _, d := range dirs {
		_ = d
	}
	d := t.TempDir()
	a, b := f()
	b = t.TempDir()
	var c string
	c = t.TempDir()
	e := t.TempDir()
	fill(&e)
	g := t.TempDir()
	g += "/x"
	ok := t.TempDir()
	if ok == "" || ok != g {
	}
	fn := func(lit string) {}
	lit := t.TempDir()
	_ = fn
	x := cfg.TempDir()
	shared = t.TempDir()
}
"#;
        // `shared` is only assigned in TestA, never declared there: it is a
        // package-level variable, which another file can assign too.
        assert_eq!(roots(src), ["2:ok"]);
    }

    #[test]
    fn scopes_are_functions() {
        let src = r#"package p
func TestA(t *testing.T) {
	dir := t.TempDir()
	t.Run("x", func(t *testing.T) {
		inner := filepath.Join(dir, "a")
	})
}
func TestB(t *testing.T) {
	_ = dir
}
"#;
        let (toks, _) = tokenize(src.as_bytes());
        let r = TempRoots::of(src.as_bytes(), &toks);
        let at = |needle: &str, nth: usize| {
            toks.iter()
                .enumerate()
                .filter(|(_, t)| t.text(src.as_bytes()) == needle.as_bytes())
                .nth(nth)
                .unwrap()
                .0
        };
        assert!(r.contains(at("dir", 0), b"dir"));
        assert!(r.contains(at("dir", 1), b"dir"));
        assert!(r.contains(at("inner", 0), b"inner"));
        // The same name in another function is not the temp root.
        assert!(!r.contains(at("dir", 2), b"dir"));
        assert!(r.is_handle(b"t"));
        assert!(!r.is_handle(b"dir"));
    }
}
