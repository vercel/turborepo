//! A small, never-executing shell-word parser for task command lines.
//!
//! It turns a command line (a `package.json` script, a Makefile recipe line, a
//! CI `run:` step) into simple [`Command`]s:
//!
//! - quoting (`'…'`, `"…"`, `\x`, line continuations) and comments;
//! - lists and pipelines: `&&`, `||`, `;`, `|`, `|&`, `&`, newlines; `( … )`
//!   subshells (whose `cd` does not leak) and `{ …; }` groups;
//! - `VAR=x cmd` prefixes ([`Command::assigns`]); bare `VAR=x` and `export
//!   VAR=x` update the environment for later words;
//! - `cd dir` / `pushd dir` / `popd` track the working directory
//!   ([`Command::cwd`]); they are not reported as commands;
//! - redirections `<`, `>`, `>>`, `2>`, `2>>`, `&>`, `<<<`, `N>&M`
//!   ([`Command::redirects`]);
//! - `$VAR`, `${VAR}`, `${VAR:-default}` expanded **only** from the provided
//!   environment.
//!
//! Anything else is not modelled and becomes a
//! [`UncertaintyKind::ShellUnsupported`] entry in [`Parsed::uncertainties`]:
//! unknown variables, command substitution (`$(…)`, backticks), arithmetic,
//! loops, conditionals, `case`, functions, here-documents, `eval`/`source`.
//! Words that contain such parts are kept with [`Word::dynamic`] set; commands
//! inside loops and conditionals are still reported (they may run).
//!
//! The parser never panics on any input and never executes anything.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::spec::{Command, Connector, Redirect, TaskUncertainty, Word};

/// Known variable values for expansion. Variables not in the map stay
/// unexpanded.
pub type Env = BTreeMap<String, String>;

/// The result of [`parse`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Simple commands in source order (`cd`, `pushd`, `popd`, `export`,
    /// `true`, `:` and bare assignments are consumed, not reported).
    pub commands: Vec<Command>,
    /// Unsupported constructs ([`UncertaintyKind::ShellUnsupported`]),
    /// deduplicated.
    pub uncertainties: Vec<TaskUncertainty>,
    /// The working directory after the last command (for multi-line recipes
    /// that keep a shell, such as `.ONESHELL`).
    pub final_cwd: PathBuf,
}

/// Parses `src` with working directory `cwd` (absolute) and known variables
/// `env`.
pub fn parse(src: &str, cwd: &Path, env: &Env) -> Parsed {
    let mut p = Parser {
        lx: Lexer {
            s: src.chars().collect(),
            i: 0,
            issues: Vec::new(),
            heredocs: Vec::new(),
            depth: 0,
            arrays: BTreeMap::new(),
        },
        env: env.clone(),
        out: Vec::new(),
        cwd_unknown: false,
    };
    let mut cwd = cwd.to_path_buf();
    let mut stack = Vec::new();
    p.list(&mut cwd, &mut stack, 0);
    let mut uncertainties: Vec<TaskUncertainty> =
        p.lx.issues
            .into_iter()
            .map(|d| TaskUncertainty::new(UncertaintyKind::ShellUnsupported, d))
            .collect();
    uncertainties.sort();
    uncertainties.dedup();
    Parsed {
        commands: p.out,
        uncertainties,
        final_cwd: cwd,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word {
        word: Word,
        /// `NAME=` prefix (unquoted, valid name): the name and the value word.
        assign: Option<(String, Word)>,
        start: usize,
        end: usize,
        /// Entirely unquoted (keywords are recognized only then).
        bare: bool,
    },
    Op(Connector, usize),
    /// Newline or `;` (a sequence point).
    Sep(usize),
    Redir(String, usize),
    /// `<(` or `>(`: a process substitution (the parenthesis is consumed).
    ProcSubst(usize),
    LParen(usize),
    RParen(usize),
    Eof,
}

struct Lexer {
    s: Vec<char>,
    i: usize,
    issues: Vec<String>,
    /// Here-document delimiters whose bodies start after the next newline.
    heredocs: Vec<String>,
    /// Nesting of command substitutions being evaluated
    /// ([`Lexer::directory_of`]).
    depth: usize,
    /// Arrays assigned so far (`NAME=( … )`) with their words: one list per
    /// literal the name was given (branches assign different ones; a
    /// conditional append gives one with and one without). `None` once the
    /// contents are not known (an element assignment, an append to an
    /// unknown array, too many literals).
    arrays: BTreeMap<String, Option<Vec<Vec<Word>>>>,
}

const MAX_NEST: usize = 64;
/// Maximum nesting of evaluated command substitutions.
const MAX_SUBST: usize = 4;
/// Maximum number of literals tracked for one array name.
const MAX_ARRAY_LITERALS: usize = 8;

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn text(&self, a: usize, b: usize) -> String {
        self.s
            .get(a.min(self.s.len())..b.min(self.s.len()))
            .map(|c| c.iter().collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn issue(&mut self, d: impl Into<String>) {
        self.issues.push(d.into());
    }

    /// Skips blanks, line continuations and comments; not newlines.
    fn skip_blank(&mut self) {
        loop {
            match self.peek() {
                // Every whitespace `is_word_end` stops at, except the newline (a command
                // separator); otherwise a vertical tab, form feed, NBSP or U+2028 yields empty
                // words forever.
                Some(c) if c != '\n' && c.is_whitespace() => self.i += 1,
                Some('\\') if self.peek_at(1) == Some('\n') => self.i += 2,
                Some('#') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.i += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn next(&mut self, env: &Env) -> Tok {
        self.skip_blank();
        let start = self.i;
        let Some(c) = self.peek() else {
            return Tok::Eof;
        };
        let two = |l: &Lexer, s: &str| {
            let mut it = s.chars();
            it.next() == l.peek() && it.next() == l.peek_at(1)
        };
        match c {
            '\n' => {
                self.i += 1;
                self.skip_heredoc_bodies();
                return Tok::Sep(start);
            }
            ';' => {
                if two(self, ";;") {
                    self.i += 2;
                } else {
                    self.i += 1;
                }
                return Tok::Sep(start);
            }
            '&' if two(self, "&&") => {
                self.i += 2;
                return Tok::Op(Connector::And, start);
            }
            '&' if two(self, "&>") => {
                self.i += 2;
                if self.peek() == Some('>') {
                    self.i += 1;
                    return Tok::Redir("&>>".into(), start);
                }
                return Tok::Redir("&>".into(), start);
            }
            '&' => {
                self.i += 1;
                return Tok::Op(Connector::Background, start);
            }
            '|' if two(self, "||") => {
                self.i += 2;
                return Tok::Op(Connector::Or, start);
            }
            '|' => {
                self.i += 1;
                if self.peek() == Some('&') {
                    self.i += 1;
                }
                return Tok::Op(Connector::Pipe, start);
            }
            '(' => {
                self.i += 1;
                if self.peek() == Some('(') {
                    // `(( … ))` arithmetic command.
                    self.issue("arithmetic command `(( … ))`");
                    self.skip_balanced('(', ')');
                    return self.next(env);
                }
                return Tok::LParen(start);
            }
            ')' => {
                self.i += 1;
                return Tok::RParen(start);
            }
            '<' | '>' => return self.redir(env, start, String::new()),
            '0'..='9' => {
                // `2>file`, `2>&1`.
                let mut j = self.i;
                while self.s.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    j += 1;
                }
                if matches!(self.s.get(j), Some('<' | '>')) {
                    let fd: String = self.s[self.i..j].iter().collect();
                    self.i = j;
                    return self.redir(env, start, fd);
                }
            }
            _ => {}
        }
        self.word(env, start)
    }

    fn redir(&mut self, env: &Env, start: usize, fd: String) -> Tok {
        let mut op = fd;
        let c = self.peek().unwrap_or('>');
        self.i += 1;
        op.push(c);
        if c == '<' && self.peek() == Some('<') {
            self.i += 1;
            if self.peek() == Some('<') {
                self.i += 1;
                op.push_str("<<");
            } else {
                self.issue("here-document");
                self.heredoc_delim();
                return self.next(env);
            }
        } else if c == '>' && self.peek() == Some('>') {
            self.i += 1;
            op.push('>');
        } else if c == '<' && self.peek() == Some('(') || c == '>' && self.peek() == Some('(') {
            self.issue("process substitution");
            self.i += 1;
            return Tok::ProcSubst(start);
        }
        if self.peek() == Some('&') {
            self.i += 1;
            op.push('&');
        } else if self.peek() == Some('|') && op.ends_with('>') {
            self.i += 1;
        }
        Tok::Redir(op, start)
    }

    /// Reads a here-document delimiter; the body is skipped at the next
    /// newline.
    fn heredoc_delim(&mut self) {
        if self.peek() == Some('-') {
            self.i += 1;
        }
        self.skip_blank();
        let mut delim = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')') {
                break;
            }
            if c != '\'' && c != '"' && c != '\\' {
                delim.push(c);
            }
            self.i += 1;
        }
        self.heredocs.push(delim);
    }

    /// After a newline: skips pending here-document bodies.
    fn skip_heredoc_bodies(&mut self) {
        for delim in std::mem::take(&mut self.heredocs) {
            loop {
                let ls = self.i;
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.i += 1;
                }
                let line: String = self.s.get(ls..self.i).unwrap_or(&[]).iter().collect();
                if self.peek().is_none() {
                    return;
                }
                self.i += 1;
                if line.trim() == delim {
                    break;
                }
            }
        }
    }

    /// Skips to the char after the matching `close` (the opener is already
    /// consumed), honoring quotes and nesting.
    fn skip_balanced(&mut self, open: char, close: char) {
        let mut depth = 1usize;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '\\' => self.i += 1,
                '\'' => {
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\'' {
                            break;
                        }
                    }
                }
                '"' => {
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\\' {
                            self.i += 1;
                        } else if c == '"' {
                            break;
                        }
                    }
                }
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
        self.i = self.i.min(self.s.len());
    }

    /// Skips an array literal to the char after its closing `)` (the opener is
    /// already consumed): quotes, nested parentheses and `#` comments are
    /// honored, so a newline or an operator between its elements does not
    /// end it.
    fn skip_array(&mut self) {
        let mut depth = 1usize;
        let mut word_start = true;
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '\\' => self.i += 1,
                '\'' => {
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\'' {
                            break;
                        }
                    }
                }
                '"' => {
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == '\\' {
                            self.i += 1;
                        } else if c == '"' {
                            break;
                        }
                    }
                }
                '#' if word_start => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            word_start = c.is_whitespace();
        }
        self.i = self.i.min(self.s.len());
    }

    /// The words of an array literal's inside, or `None` when it holds anything
    /// but words.
    fn array_words(&mut self, inner: &str, env: &Env) -> Option<Vec<Word>> {
        let mut lx = Lexer {
            s: inner.chars().collect(),
            i: 0,
            issues: Vec::new(),
            heredocs: Vec::new(),
            depth: self.depth + 1,
            arrays: BTreeMap::new(),
        };
        let mut words = Vec::new();
        loop {
            match lx.next(env) {
                Tok::Word { word, .. } => words.push(word),
                Tok::Sep(_) => {}
                Tok::Eof => return Some(words),
                _ => return None,
            }
        }
    }

    /// At the `'` of `$'…'` (ANSI-C quoting): the decoded text up to the
    /// closing quote.
    fn ansi_c(&mut self, w: &mut Word) {
        self.i += 1;
        loop {
            match self.peek() {
                None => {
                    self.issue("unterminated quote");
                    return;
                }
                Some('\'') => {
                    self.i += 1;
                    return;
                }
                Some('\\') => {
                    self.i += 1;
                    let Some(e) = self.peek() else { return };
                    self.i += 1;
                    w.text.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'e' | 'E' => '\u{1b}',
                        '0' => '\0',
                        c => c,
                    });
                }
                Some(c) => {
                    w.text.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// The directory a command substitution prints, when it is one of the
    /// idioms that derive a directory from a known absolute one: `cd DIR &&
    /// pwd` (also `;`, `-P`, `--`, output redirections) and `dirname PATH`.
    /// `None` for anything else, and when `DIR` is relative or not known
    /// (the substitution's own working directory is not).
    fn directory_of(&mut self, inner: &str, env: &Env) -> Option<String> {
        let t = inner.trim_start();
        if !(t.starts_with("cd ") || t.starts_with("dirname ")) || self.depth >= MAX_SUBST {
            return None;
        }
        // A start directory no real path is below: a result below it is relative.
        const UNKNOWN_ROOT: &str = "/\u{1}unknown-cwd";
        const UNKNOWN: &str = "/\u{1}unknown-cwd/1/2/3/4/5/6/7/8/9/10/11/12/13/14/15/16";
        let mut p = Parser {
            lx: Lexer {
                s: inner.chars().collect(),
                i: 0,
                issues: Vec::new(),
                heredocs: Vec::new(),
                depth: self.depth + 1,
                arrays: BTreeMap::new(),
            },
            env: env.clone(),
            out: Vec::new(),
            cwd_unknown: false,
        };
        let mut cwd = PathBuf::from(UNKNOWN);
        p.list(&mut cwd, &mut Vec::new(), 0);
        if !p.lx.issues.is_empty() {
            return None;
        }
        let [c] = p.out.as_slice() else {
            return None;
        };
        let known =
            |d: &Path| d.is_absolute() && !d.starts_with(UNKNOWN_ROOT) && d != Path::new("/");
        match c.program()? {
            "pwd" if c.argv[1..].iter().all(|w| w.text.starts_with('-')) && known(&c.cwd) => {
                Some(c.cwd.to_string_lossy().into_owned())
            }
            "dirname" => {
                let mut args = c.argv[1..].iter().filter(|w| w.text != "--");
                let (Some(a), None) = (args.next(), args.next()) else {
                    return None;
                };
                let path = fsx::normalize(Path::new(&a.text));
                if a.dynamic || a.glob || !known(&path) {
                    return None;
                }
                Some(path.parent()?.to_string_lossy().into_owned())
            }
            _ => None,
        }
    }

    fn is_word_end(c: char) -> bool {
        c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')')
    }

    fn word(&mut self, env: &Env, start: usize) -> Tok {
        let mut w = Word::default();
        let mut bare = true;
        let mut assign: Option<String> = None;
        // The assignment does not give the variable a value this parser knows: an
        // append (`NAME+=x`), an array element (`NAME[i]=x`) or an array
        // (`NAME=(…)`).
        let mut opaque = false;
        // Assignment prefix: NAME=, NAME+= or NAME[index]= with NAME unquoted.
        {
            let mut j = self.i;
            if self
                .s
                .get(j)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_')
            {
                while self
                    .s
                    .get(j)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    j += 1;
                }
                let mut k = j;
                if self.s.get(k) == Some(&'[') {
                    // An index: up to the `]` on the same word.
                    let mut e = k + 1;
                    while self
                        .s
                        .get(e)
                        .is_some_and(|c| *c != ']' && !c.is_whitespace())
                    {
                        e += 1;
                    }
                    if self.s.get(e) == Some(&']') {
                        k = e + 1;
                    }
                }
                if self.s.get(k) == Some(&'+') && self.s.get(k + 1) == Some(&'=') {
                    k += 1;
                }
                if self.s.get(k) == Some(&'=') {
                    assign = Some(self.s[self.i..j].iter().collect());
                    opaque = k != j;
                    self.i = k + 1;
                }
            }
        }
        if assign.is_some() && self.peek() == Some('(') {
            // `NAME=( … )`: an array. Its elements are words, not commands; the parser
            // does not track array values.
            let a = self.i;
            self.i += 1;
            self.skip_array();
            let name = assign.unwrap_or_default();
            let inner: String = self
                .s
                .get(a + 1..self.i.saturating_sub(1).max(a + 1))
                .unwrap_or(&[])
                .iter()
                .collect();
            let words = if self.depth < MAX_SUBST {
                self.array_words(&inner, env)
            } else {
                None
            };
            let append = a >= 2 && self.s.get(a - 2) == Some(&'+');
            let known = match (opaque, self.arrays.remove(&name), words) {
                // `NAME=( … )`: the first literal, or one more.
                (false, None, Some(w)) => Some(vec![w]),
                (false, Some(Some(mut alts)), Some(w)) => {
                    if !alts.contains(&w) {
                        alts.push(w);
                    }
                    Some(alts)
                }
                // `NAME+=( … )` to an array whose words are known: with and without it.
                (true, Some(Some(mut alts)), Some(w)) if append => {
                    let longer: Vec<Vec<Word>> = alts
                        .iter()
                        .map(|a| a.iter().chain(&w).cloned().collect())
                        .collect();
                    for l in longer {
                        if !alts.contains(&l) {
                            alts.push(l);
                        }
                    }
                    Some(alts)
                }
                _ => None,
            }
            .filter(|alts| alts.len() <= MAX_ARRAY_LITERALS);
            self.arrays.insert(name.clone(), known);
            let mut value = Word::lit(self.s[a..self.i].iter().collect::<String>());
            value.dynamic = true;
            let mut whole = Word::lit(format!("{name}={}", value.text));
            whole.dynamic = true;
            return Tok::Word {
                word: whole,
                assign: Some((name, value)),
                start,
                end: self.i,
                bare: false,
            };
        }
        let first = self.i;
        while let Some(c) = self.peek() {
            if Self::is_word_end(c) {
                // `${…}` / `$(…)` are consumed by `dollar`, so a terminator here ends the word.
                break;
            }
            match c {
                '\'' => {
                    bare = false;
                    self.i += 1;
                    loop {
                        match self.peek() {
                            None => {
                                self.issue("unterminated quote");
                                break;
                            }
                            Some('\'') => {
                                self.i += 1;
                                break;
                            }
                            Some(c) => {
                                w.text.push(c);
                                self.i += 1;
                            }
                        }
                    }
                }
                '"' => {
                    bare = false;
                    self.i += 1;
                    loop {
                        match self.peek() {
                            None => {
                                self.issue("unterminated quote");
                                break;
                            }
                            Some('"') => {
                                self.i += 1;
                                break;
                            }
                            Some('\\') => {
                                let n = self.peek_at(1);
                                match n {
                                    Some('\n') => self.i += 2,
                                    Some(c @ ('$' | '`' | '"' | '\\')) => {
                                        w.text.push(c);
                                        self.i += 2;
                                    }
                                    _ => {
                                        w.text.push('\\');
                                        self.i += 1;
                                    }
                                }
                            }
                            Some('$') => self.dollar(env, &mut w, true),
                            Some('`') => self.backtick(&mut w),
                            Some(c) => {
                                w.text.push(c);
                                self.i += 1;
                            }
                        }
                    }
                }
                '\\' => {
                    bare = false;
                    match self.peek_at(1) {
                        Some('\n') => self.i += 2,
                        Some(c) => {
                            w.text.push(c);
                            self.i += 2;
                        }
                        None => self.i += 1,
                    }
                }
                '$' => self.dollar(env, &mut w, false),
                '`' => self.backtick(&mut w),
                '*' | '?' | '[' => {
                    w.glob = true;
                    w.text.push(c);
                    self.i += 1;
                }
                '~' if self.i == first => {
                    match env.get("HOME") {
                        Some(h) => w.text.push_str(h),
                        None => {
                            w.dynamic = true;
                            w.text.push('~');
                        }
                    }
                    self.i += 1;
                }
                c => {
                    w.text.push(c);
                    self.i += 1;
                }
            }
        }
        let end = self.i;
        w.dynamic |= opaque;
        if let Some(name) = assign.as_ref().filter(|_| opaque) {
            // `NAME[i]=x`, `NAME+=x`: whatever array it was is no longer known.
            if let Some(a) = self.arrays.get_mut(name) {
                *a = None;
            }
        }
        match assign {
            Some(name) => {
                let mut whole = Word::lit(format!("{name}={}", w.text));
                whole.dynamic = w.dynamic;
                Tok::Word {
                    word: whole,
                    assign: Some((name, w)),
                    start,
                    end,
                    bare,
                }
            }
            None => Tok::Word {
                word: w,
                assign: None,
                start,
                end,
                bare,
            },
        }
    }

    fn backtick(&mut self, w: &mut Word) {
        self.issue("command substitution (backticks)");
        let a = self.i;
        self.i += 1;
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == '\\' {
                self.i += 1;
            } else if c == '`' {
                break;
            }
        }
        self.i = self.i.min(self.s.len());
        w.text
            .push_str(&self.s[a..self.i].iter().collect::<String>());
        w.dynamic = true;
    }

    /// At a `$`: expands a known variable or keeps the source and marks the
    /// word dynamic.
    fn dollar(&mut self, env: &Env, w: &mut Word, quoted: bool) {
        let a = self.i;
        self.i += 1;
        match self.peek() {
            Some('(') => {
                self.i += 1;
                let arithmetic = self.peek() == Some('(');
                let b = self.i;
                self.skip_balanced('(', ')');
                if !arithmetic {
                    let inner: String = self
                        .s
                        .get(b..self.i.saturating_sub(1).max(b))
                        .unwrap_or(&[])
                        .iter()
                        .collect();
                    if let Some(dir) = self.directory_of(&inner, env) {
                        w.text.push_str(&dir);
                        return;
                    }
                }
                if arithmetic {
                    self.issue("arithmetic expansion");
                } else {
                    self.issue("command substitution `$(…)`");
                }
                w.text
                    .push_str(&self.s[a..self.i].iter().collect::<String>());
                w.dynamic = true;
            }
            Some('\'') if !quoted => self.ansi_c(w),
            Some('{') => {
                self.i += 1;
                let b = self.i;
                self.skip_balanced('{', '}');
                let inner: String = self
                    .s
                    .get(b..self.i.saturating_sub(1).max(b))
                    .unwrap_or(&[])
                    .iter()
                    .collect();
                let name_end = inner
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(inner.len());
                let name = &inner[..name_end];
                let rest = &inner[name_end..];
                let val = env.get(name).cloned();
                match (val, rest) {
                    (Some(v), "") => w.text.push_str(&v),
                    (Some(v), r) if r.starts_with(":-") || r.starts_with('-') => {
                        if v.is_empty() && r.starts_with(":-") {
                            w.text.push_str(r.trim_start_matches(":-"));
                        } else {
                            w.text.push_str(&v);
                        }
                    }
                    _ => {
                        self.issue(format!("unknown variable `${{{inner}}}`"));
                        w.text
                            .push_str(&self.s[a..self.i].iter().collect::<String>());
                        w.dynamic = true;
                    }
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let b = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.i += 1;
                }
                let name: String = self.s[b..self.i].iter().collect();
                match env.get(&name) {
                    Some(v) => w.text.push_str(v),
                    None => {
                        self.issue(format!("unknown variable `${name}`"));
                        w.text.push('$');
                        w.text.push_str(&name);
                        w.dynamic = true;
                    }
                }
            }
            Some(c)
                if c.is_ascii_digit() || matches!(c, '@' | '*' | '#' | '?' | '$' | '!' | '-') =>
            {
                self.i += 1;
                self.issue(format!("special parameter `${c}`"));
                w.text.push('$');
                w.text.push(c);
                w.dynamic = true;
            }
            _ => w.text.push('$'),
        }
    }
}

struct Parser {
    lx: Lexer,
    env: Env,
    out: Vec<Command>,
    /// The working directory is not known: a `cd` to a computed directory came
    /// before.
    cwd_unknown: bool,
}

const KEYWORDS_SKIP: &[&str] = &[
    "then", "do", "else", "fi", "done", "esac", "in", "!", "{", "}",
];

impl Parser {
    /// Parses a list until EOF or (at depth > 0) a closing paren.
    fn list(&mut self, cwd: &mut PathBuf, stack: &mut Vec<PathBuf>, depth: usize) {
        let mut cur = Command {
            cwd: cwd.clone(),
            ..Default::default()
        };
        let mut start: Option<usize> = None;
        let mut end = 0usize;
        let mut pending_redir: Option<String> = None;
        // Words to skip until the next separator (`for x in …`).
        let mut skip_to_sep = false;
        let mut in_case = false;
        // Inside `[[ … ]]`: `&&`, `||`, `<`, `>` and parentheses are operators of the
        // test, not of the shell, so its words stay one command.
        let mut in_test = false;
        let mut after_op = false;
        loop {
            let tok = self.lx.next(&self.env);
            if in_test {
                let was_op = std::mem::replace(&mut after_op, matches!(tok, Tok::Op(..)));
                match &tok {
                    Tok::Op(..) | Tok::LParen(_) | Tok::RParen(_) | Tok::Redir(..) => continue,
                    Tok::ProcSubst(_) => {}
                    // A line break continues the test only after `&&` or `||`.
                    Tok::Sep(_) if was_op => continue,
                    Tok::Sep(_) | Tok::Eof => in_test = false,
                    Tok::Word { word, bare, .. } => {
                        if *bare && word.text == "]]" {
                            in_test = false;
                        }
                    }
                }
            }
            match tok {
                Tok::Eof => {
                    self.finish(&mut cur, cwd, stack, start, end, Connector::Seq);
                    if depth > 0 {
                        self.lx.issue("unbalanced parentheses");
                    }
                    return;
                }
                Tok::RParen(_) if in_case => {}
                Tok::RParen(_) if depth > 0 => {
                    self.finish(&mut cur, cwd, stack, start, end, Connector::Seq);
                    return;
                }
                Tok::RParen(_) => self.lx.issue("unbalanced parentheses"),
                Tok::LParen(p) => {
                    if !cur.argv.is_empty() {
                        // `name() { …; }`: a function definition.
                        self.lx.issue("function definition");
                        cur.argv.clear();
                        start = None;
                        continue;
                    }
                    let _ = p;
                    if depth >= MAX_NEST {
                        self.lx.issue("nesting too deep");
                        continue;
                    }
                    // Subshell: its cwd changes don't leak.
                    let mut sub_cwd = cwd.clone();
                    let mut sub_stack = stack.clone();
                    let unknown = self.cwd_unknown;
                    self.list(&mut sub_cwd, &mut sub_stack, depth + 1);
                    self.cwd_unknown = unknown;
                }
                Tok::Sep(_) => {
                    skip_to_sep = false;
                    self.finish(&mut cur, cwd, stack, start, end, Connector::Seq);
                    start = None;
                }
                Tok::Op(c, _) => {
                    skip_to_sep = false;
                    self.finish(&mut cur, cwd, stack, start, end, c);
                    start = None;
                }
                Tok::Redir(op, p) => {
                    start.get_or_insert(p);
                    pending_redir = Some(op);
                }
                Tok::ProcSubst(p) => {
                    // `cmd <(other)`, `done < <(other)`: `other` runs, and the word is
                    // the name of a pipe (never a file of the repository).
                    start.get_or_insert(p);
                    let redirected = pending_redir.take().is_some();
                    if depth >= MAX_NEST {
                        self.lx.issue("nesting too deep");
                        continue;
                    }
                    let mut sub_cwd = cwd.clone();
                    let mut sub_stack = stack.clone();
                    let unknown = self.cwd_unknown;
                    self.list(&mut sub_cwd, &mut sub_stack, depth + 1);
                    self.cwd_unknown = unknown;
                    if !redirected && !cur.argv.is_empty() {
                        let mut w = Word::lit("<(…)");
                        w.dynamic = true;
                        cur.argv.push(w);
                    }
                }
                Tok::Word {
                    word,
                    assign,
                    start: s,
                    end: e,
                    bare,
                } => {
                    if let Some(op) = pending_redir.take() {
                        // `2>&1`-style: the word after `>&` is a descriptor.
                        let mut target = word;
                        if op.ends_with('&') {
                            target.text = format!("&{}", target.text);
                        }
                        cur.redirects.push(Redirect {
                            op: op.trim_end_matches('&').to_string(),
                            target,
                        });
                        end = e;
                        continue;
                    }
                    if in_case {
                        if bare && word.text == "esac" {
                            in_case = false;
                        }
                        continue;
                    }
                    if skip_to_sep {
                        continue;
                    }
                    // `"${NAME[@]}"`: the words of the array, when it was assigned as a
                    // literal (`cmd=(tool --flag); "${cmd[@]}" file`).
                    if assign.is_none()
                        && let Some(mut alts) = self.array_expansion(&word)
                        && (alts.len() == 1 || cur.argv.is_empty())
                        && let Some(last) = alts.pop()
                    {
                        // An array given several literals, expanded as the command: the
                        // last one takes the arguments that follow, the others run too.
                        for argv in alts {
                            let text = argv
                                .iter()
                                .map(|w| w.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" ");
                            self.out.push(Command {
                                argv,
                                cwd: cwd.clone(),
                                cwd_computed: self.cwd_unknown,
                                text,
                                ..Default::default()
                            });
                        }
                        start.get_or_insert(s);
                        end = e;
                        cur.argv.extend(last);
                        continue;
                    }
                    let at_cmd_start = cur.argv.is_empty();
                    if at_cmd_start && bare {
                        let t = word.text.as_str();
                        match t {
                            "if" | "elif" | "while" | "until" => {
                                self.lx.issue(format!("conditional or loop `{t}`"));
                                continue;
                            }
                            "for" | "select" => {
                                self.lx.issue(format!("loop `{t}`"));
                                skip_to_sep = true;
                                continue;
                            }
                            "case" => {
                                self.lx.issue("conditional `case`");
                                in_case = true;
                                continue;
                            }
                            "function" => {
                                self.lx.issue("function definition");
                                skip_to_sep = true;
                                continue;
                            }
                            _ if KEYWORDS_SKIP.contains(&t) => continue,
                            "[[" => in_test = true,
                            _ => {}
                        }
                    }
                    // `NAME=value` prefixes; the value may be quoted (`NODE_OPTIONS='--x y' tsc`).
                    if at_cmd_start && let Some((name, val)) = assign {
                        start.get_or_insert(s);
                        end = e;
                        cur.assigns.push((name, val));
                        continue;
                    }
                    start.get_or_insert(s);
                    end = e;
                    cur.argv.push(word);
                }
            }
        }
    }

    /// The literals `word` may expand to when it is exactly `${NAME[@]}` (or
    /// `[*]`) of an array whose literals are known and not empty.
    fn array_expansion(&self, word: &Word) -> Option<Vec<Vec<Word>>> {
        if !word.dynamic {
            return None;
        }
        let name = word
            .text
            .strip_prefix("${")
            .and_then(|t| t.strip_suffix("[@]}").or_else(|| t.strip_suffix("[*]}")))?;
        let alts = self.lx.arrays.get(name)?.as_ref()?;
        (!alts.is_empty() && alts.iter().all(|a| !a.is_empty())).then(|| alts.clone())
    }

    /// Completes the current command: handles builtins that change state, or
    /// pushes it.
    fn finish(
        &mut self,
        cur: &mut Command,
        cwd: &mut PathBuf,
        stack: &mut Vec<PathBuf>,
        start: Option<usize>,
        end: usize,
        next: Connector,
    ) {
        let mut c = std::mem::take(cur);
        cur.cwd = cwd.clone();
        if c.argv.is_empty() && c.redirects.is_empty() {
            // Bare assignments update the environment.
            for (k, v) in c.assigns {
                if v.dynamic {
                    self.env.remove(&k);
                } else {
                    self.env.insert(k, v.text);
                }
            }
            return;
        }
        c.next = next;
        c.cwd_computed = self.cwd_unknown;
        c.text = start.map(|s| self.lx.text(s, end)).unwrap_or_default();
        let prog = c.program().unwrap_or("").to_string();
        match prog.as_str() {
            "cd" | "pushd" => {
                let arg = c.argv.iter().skip(1).find(|w| !w.text.starts_with('-'));
                match arg {
                    Some(w) if w.is_static() && !w.glob => {
                        if prog == "pushd" {
                            stack.push(cwd.clone());
                        }
                        if Path::new(&w.text).is_absolute() {
                            self.cwd_unknown = false;
                        }
                        *cwd = fsx::join(cwd, &w.text);
                    }
                    Some(w) => {
                        self.cwd_unknown = true;
                        self.lx
                            .issue(format!("`{prog}` to a computed directory `{}`", w.text))
                    }
                    None => self.lx.issue(format!("`{prog}` without a directory")),
                }
                cur.cwd = cwd.clone();
            }
            "popd" => {
                match stack.pop() {
                    Some(d) => *cwd = d,
                    None => self.lx.issue("`popd` with an empty stack"),
                }
                cur.cwd = cwd.clone();
            }
            "export" => {
                for w in c.argv.iter().skip(1) {
                    if let Some((k, v)) = w.text.split_once('=') {
                        if w.dynamic {
                            self.env.remove(k);
                        } else {
                            self.env.insert(k.to_string(), v.to_string());
                        }
                    }
                }
            }
            "set" | "true" | ":" | "shopt" | "unset" | "trap" | "return" | "exit" => {}
            "eval" => self.lx.issue("`eval`"),
            "source" | "." => {
                self.lx.issue(format!("`{prog}` of a shell file"));
                self.out.push(c);
            }
            _ => self.out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Parsed {
        parse(s, Path::new("/r/pkg"), &Env::new())
    }

    fn argv(c: &Command) -> Vec<&str> {
        c.argv.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn lists_and_quotes() {
        let r = p(r#"tsc -p "tsconfig build.json" && echo 'a b' || x\ y; c | d"#);
        assert_eq!(r.commands.len(), 5);
        assert_eq!(argv(&r.commands[0]), ["tsc", "-p", "tsconfig build.json"]);
        assert_eq!(r.commands[0].next, Connector::And);
        assert_eq!(argv(&r.commands[1]), ["echo", "a b"]);
        assert_eq!(r.commands[1].next, Connector::Or);
        assert_eq!(argv(&r.commands[2]), ["x y"]);
        assert_eq!(r.commands[3].next, Connector::Pipe);
        assert!(r.uncertainties.is_empty());
        assert_eq!(r.commands[0].text, r#"tsc -p "tsconfig build.json""#);
    }

    #[test]
    fn quoted_assignment_values() {
        let r = p("NODE_OPTIONS='--max-old-space-size=8192' A=\"x y\" tsc --noEmit");
        let c = &r.commands[0];
        assert_eq!(c.program_name(), Some("tsc"));
        assert_eq!(c.assigns[0].1.text, "--max-old-space-size=8192");
        assert_eq!(c.assigns[1].1.text, "x y");
        // A quoted assignment-looking word after the program is an argument.
        let r = p("env 'A=b' tsc");
        assert_eq!(r.commands[0].argv.len(), 3);
    }

    #[test]
    fn assigns_and_env() {
        let mut env = Env::new();
        env.insert("OUT".into(), "dist".into());
        let r = parse(
            "NODE_ENV=production A=$OUT node build.js ${OUT}/x $MISSING",
            Path::new("/r"),
            &env,
        );
        let c = &r.commands[0];
        assert_eq!(c.assigns[0].0, "NODE_ENV");
        assert_eq!(c.assigns[1].1.text, "dist");
        assert_eq!(argv(c), ["node", "build.js", "dist/x", "$MISSING"]);
        assert!(c.argv[3].dynamic);
        assert_eq!(r.uncertainties.len(), 1);
        // Bare assignments carry over.
        let r = p("X=lib; tsc --outDir $X");
        assert_eq!(argv(&r.commands[0]), ["tsc", "--outDir", "lib"]);
    }

    #[test]
    fn cd_tracking() {
        let r = p("cd packages/a && tsc && (cd ../b && make) && pwd");
        assert_eq!(r.commands[0].cwd, PathBuf::from("/r/pkg/packages/a"));
        assert_eq!(r.commands[1].cwd, PathBuf::from("/r/pkg/packages/b"));
        assert_eq!(r.commands[2].cwd, PathBuf::from("/r/pkg/packages/a"));
        assert_eq!(r.final_cwd, PathBuf::from("/r/pkg/packages/a"));
    }

    #[test]
    fn redirections() {
        let r = p("node gen.js < in.json > out.ts 2>/dev/null 2>&1 >> log.txt");
        let c = &r.commands[0];
        assert_eq!(argv(c), ["node", "gen.js"]);
        let rs: Vec<_> = c
            .redirects
            .iter()
            .map(|r| (r.op.as_str(), r.target.text.as_str()))
            .collect();
        assert_eq!(
            rs,
            [
                ("<", "in.json"),
                (">", "out.ts"),
                ("2>", "/dev/null"),
                ("2>", "&1"),
                (">>", "log.txt")
            ]
        );
    }

    #[test]
    fn unsupported() {
        let r = p("for f in *.ts; do tsc $f; done");
        assert_eq!(argv(&r.commands[0]), ["tsc", "$f"]);
        assert!(!r.uncertainties.is_empty());
        let r = p("echo $(git rev-parse HEAD) `date`");
        assert!(r.commands[0].argv[1].dynamic);
        assert!(r.commands[0].argv[2].dynamic);
        assert_eq!(r.uncertainties.len(), 2);
        let r = p("if [ -f x ]; then a; else b; fi");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["[", "a", "b"]);
        let r = p("eval $X");
        assert!(r.commands.is_empty());
        let r = p("case $x in a) foo;; esac; bar");
        assert_eq!(r.commands.len(), 1);
        let r = p("f() { a; }; f");
        assert!(
            r.uncertainties
                .iter()
                .any(|u| u.detail.contains("function"))
        );
    }

    /// An array literal is one assignment: its elements, on one line or
    /// several, are words, never commands; an append and an element
    /// assignment are assignments too.
    #[test]
    fn arrays_and_appends_are_assignments() {
        let r =
            p("dirs=(\n  \"node_modules/\"\n  \".git/\" # it's metadata\n  dist/\n)\nrun ./a.sh");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["run"]);
        let r = p("failed+=(\"$svc: has no src/ directory\")\nlist+=(\"$DIR/x-${id}\"); next");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["next"]);
        let r = p("msg+=$'\\n'\"  - <$URL/$ws/x|$label>\"\nm[1]=a/b\nnext");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["next"]);
        // The variable no longer has a value the parser knows.
        let r = p("X=lib; X+=/sub; tsc --outDir $X");
        assert!(r.commands[0].argv[2].dynamic);
    }

    /// A command kept in an array runs where the array is expanded.
    #[test]
    fn an_array_expands_to_its_words() {
        let r = p(
            "local cmd=(\n  gotestsum\n  --format testdox\n  --\n  \"$1\"\n)\n\"${cmd[@]}\" ./pkg"
        );
        let c = r.commands.last().unwrap();
        assert_eq!(
            argv(c),
            ["gotestsum", "--format", "testdox", "--", "$1", "./pkg"]
        );
        assert!(c.argv[4].dynamic);
        // An append may be conditional: as an argument the array is then not known.
        let r = p("args=(-t x); args+=(--build-arg \"A=1\")\ndocker build \"${args[@]}\" .");
        assert_eq!(argv(&r.commands[0]), ["docker", "build", "${args[@]}", "."]);
        let r = p("args=(-t x)\ndocker build \"${args[@]}\" .");
        assert_eq!(argv(&r.commands[0]), ["docker", "build", "-t", "x", "."]);
        // Branches give the name different commands: each one runs.
        let r = p(
            "if x; then fmt=(pnpm exec oxfmt); else fmt=(pnpm dlx \"oxfmt@$v\"); \
             fi\n\"${fmt[@]}\" --write a.yaml",
        );
        let got: Vec<Vec<&str>> = r.commands.iter().map(argv).collect();
        assert_eq!(
            got[1..],
            [
                vec!["pnpm", "exec", "oxfmt"],
                vec!["pnpm", "dlx", "oxfmt@$v", "--write", "a.yaml"]
            ]
        );
        // Not known: an element assignment, an append to an array that was never
        // assigned.
        let r = p("a=(x y); a[0]=z; b+=(q)\nrun \"${a[@]}\" \"${b[@]}\"");
        let c = &r.commands[0];
        assert_eq!(argv(c), ["run", "${a[@]}", "${b[@]}"]);
        assert!(c.argv[1].dynamic && c.argv[2].dynamic);
    }

    /// `<(cmd)` runs `cmd`; it is no function definition and no redirection
    /// from a file.
    #[test]
    fn process_substitution_runs_its_command() {
        let r = p("read -r a b < <(awk '\n  /^## / { n++ }\n  END { print n }\n' \"$f\")\nnext");
        let got: Vec<Vec<&str>> = r.commands.iter().map(argv).collect();
        assert_eq!(got[0][0], "awk");
        assert_eq!(got[1], ["read", "-r", "a", "b"]);
        assert!(r.commands[1].redirects.is_empty());
        assert_eq!(got[2], ["next"]);
        let r = p("diff <(sort a.txt) <(sort b.txt) > out.txt");
        let got: Vec<Vec<&str>> = r.commands.iter().map(argv).collect();
        assert_eq!(
            got,
            [
                vec!["sort", "a.txt"],
                vec!["sort", "b.txt"],
                vec!["diff", "<(…)", "<(…)"]
            ]
        );
        let r = p("while read -r f; do x \"$f\"; done < <(find . -name '*.tf')\nnext");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["read", "x", "find", "next"]);
    }

    #[test]
    fn ansi_c_quoting() {
        let r = p("printf $'a\\tb\\n' x");
        assert_eq!(argv(&r.commands[0]), ["printf", "a\tb\n", "x"]);
        // Inside double quotes `$'` is a dollar sign and a quote.
        let r = p("echo \"cost: $'5'\" y");
        assert_eq!(argv(&r.commands[0]), ["echo", "cost: $'5'", "y"]);
    }

    /// `[[ … ]]` is one command: `&&`, `||`, `<` and parentheses inside it
    /// belong to the test.
    #[test]
    fn double_bracket_test_is_one_command() {
        let r = p("if [[ \"${A:-}\" == 1 && \"${PWD##*/}\" == site ]]; then exit 0; fi\nnext");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["[[", "next"]);
        let r = p("[[ \"$f\" =~ ^[0-9]{14}_(.+)$ ]] && a/b.sh || c");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["[[", "a/b.sh", "c"]);
        let r = p("[[ -n $a ||\n  -n $b ]]\nnext");
        let progs: Vec<_> = r.commands.iter().filter_map(|c| c.program()).collect();
        assert_eq!(progs, ["[[", "next"]);
        // An unterminated test ends at the line.
        let r = p("[[ -n $a\nnext x");
        assert_eq!(r.commands.len(), 2);
    }

    /// A substitution that derives a directory from a known absolute one has a
    /// value.
    #[test]
    fn directory_substitutions_are_evaluated() {
        let mut env = Env::new();
        env.insert("HERE".into(), "/r/pkg/scripts".into());
        let q = |s: &str| parse(s, Path::new("/r/pkg"), &env);
        let r = q(
            "ROOT=\"$(cd \"$HERE/..\" && pwd)\"\ndocker build -f \"$ROOT/c/Dockerfile\" \"$ROOT\"",
        );
        assert_eq!(
            argv(&r.commands[0]),
            ["docker", "build", "-f", "/r/pkg/c/Dockerfile", "/r/pkg"]
        );
        assert!(r.uncertainties.is_empty(), "{:?}", r.uncertainties);
        let r = q(
            "A=$(cd -- \"$HERE/../..\" >/dev/null 2>&1; pwd -P); B=$(dirname \"$HERE\"); x $A $B",
        );
        assert_eq!(argv(&r.commands[0]), ["x", "/r", "/r/pkg"]);
        // Not evaluated: a relative directory (the substitution's working directory is
        // not known here), an unknown one, and any other command.
        for s in [
            "x $(cd .. && pwd)",
            "x $(cd \"$NOPE/..\" && pwd)",
            "x $(cd \"$HERE\" && ls)",
            "x $(dirname \"$0\")",
            "x $(cd \"$HERE\" && pwd && echo y)",
        ] {
            let r = q(s);
            assert!(r.commands[0].argv[1].dynamic, "{s}");
        }
    }

    /// After a `cd` to a computed directory the working directory is not known,
    /// until the subshell ends or a `cd` to an absolute directory.
    #[test]
    fn computed_cd_marks_the_commands_after_it() {
        let r = p("a; (cd \"$d\" && go test ./b); c; cd \"$e\"; f; cd /abs; g");
        let got: Vec<_> = r
            .commands
            .iter()
            .map(|c| (c.program().unwrap_or(""), c.cwd_computed))
            .collect();
        assert_eq!(
            got,
            [
                ("a", false),
                ("go", true),
                ("c", false),
                ("f", true),
                ("g", false)
            ]
        );
    }

    #[test]
    fn heredoc_and_comments() {
        let r = p("cat <<EOF > x.txt\nhello $Y\nEOF\nnext # comment && no");
        assert_eq!(r.commands.len(), 2);
        assert_eq!(argv(&r.commands[1]), ["next"]);
    }

    #[test]
    fn never_panics() {
        // A deterministic pseudo-random fuzz over shell-ish alphabets.
        let alphabet: Vec<char> = "ab $(){}[]'\"`\\|&;<>#=*?~\n\t-0129cdx!".chars().collect();
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 40) as usize;
            let mut s = String::new();
            let mut x = seed;
            for _ in 0..len {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                s.push(alphabet[(x >> 33) as usize % alphabet.len()]);
            }
            let _ = p(&s);
        }
        for s in [
            "((", "))", "$((", "${", "`", "'", "\"", "\\", "<<", "2>&", "(((((((", "cd", "popd",
            "f(){",
        ] {
            let _ = p(s);
        }
        let deep = "(".repeat(10_000);
        let _ = p(&deep);
    }

    #[test]
    fn unicode_and_control_whitespace_terminates() {
        // Each used to loop forever: `is_word_end` stopped at them but `skip_blank`
        // didn't skip them.
        for ws in ['\u{0b}', '\u{0c}', '\u{a0}', '\u{2028}', '\u{3000}'] {
            let src = format!("tsc{ws}-p{ws}tsconfig.json");
            let parsed = p(&src);
            assert_eq!(parsed.commands.len(), 1, "{ws:?}");
            assert_eq!(parsed.commands[0].argv.len(), 3, "{ws:?}");
        }
    }
}
