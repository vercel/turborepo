//! Shell script files run by tasks (`./scripts/build.sh`, `bash x.sh`, `sh -c
//! '…'`, `source lib.sh`), shared with the `lang-shell` language plugin.
//!
//! A script a task runs is **inlined**: its file is a task entry (so the
//! `lang-shell` trace of it, including the files it sources, counts), and its
//! commands are recognized into the task's spec with the registry, as if they
//! were written in the task (with the caller's working directory, `$0` and
//! positional arguments). Sourced files are followed the same way.
//!
//! # Script-directory anchors
//!
//! Scripts locate siblings with expressions like `"$(dirname "$0")/lib.sh"`,
//! `$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)` or `${BASH_SOURCE%/*}`, the
//! shell equivalent of `__dirname`. [`rewrite_anchors`] replaces each such
//! expression with a same-length `${_TTD…}` variable reference (so byte offsets
//! are unchanged), and the caller binds those variables to the script's
//! directory before parsing.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;

use crate::{
    registry::{Recognition, RecognizeCx, Recognizer, registry},
    shell::{self, Env},
    sources::npm::worse,
    spec::{Command, EntryLang, TaskSpec, Word},
};

/// Prefix of the variables [`rewrite_anchors`] introduces.
pub const ANCHOR_PREFIX: &str = "_TTD";

/// Shells whose script argument is a shell file.
pub const SHELLS: &[&str] = &["bash", "sh", "zsh", "dash", "ksh", "ash", "mksh"];

/// Extensions of shell script files.
pub const SCRIPT_EXTS: &[&str] = &["sh", "bash", "zsh", "ksh"];

/// Shell builtins without file effects (besides their redirections, which are
/// kept).
pub const SHELL_BUILTINS: &[&str] = &[
    "local",
    "declare",
    "typeset",
    "readonly",
    "read",
    "shift",
    "getopts",
    "let",
    "mapfile",
    "readarray",
    "alias",
    "unalias",
    "hash",
    "umask",
    "ulimit",
    "caller",
    "compgen",
    "complete",
    "compopt",
    "enable",
    "help",
    "history",
    "jobs",
    "disown",
    "bind",
    "dirs",
    "fg",
    "bg",
    "times",
    "break",
    "continue",
    "return",
    "exit",
    "wait",
    "trap",
    "set",
    "unset",
    "shopt",
    "true",
    "false",
    ":",
    "printf",
    "echo",
    "export",
    "type",
    "logout",
    "suspend",
    "setopt",
    "unsetopt",
    "autoload",
    "zmodload",
    "emulate",
];

/// Names of shell functions defined in `text` (`name() {`, `function name`).
pub fn functions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let ident = |c: &char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.');
    for line in text.lines() {
        let t = line.trim_start();
        let name: String = match t.strip_prefix("function ") {
            Some(r) => r.trim_start().chars().take_while(ident).collect(),
            None => {
                let n: String = t.chars().take_while(ident).collect();
                if !n.is_empty() && t[n.len()..].trim_start().starts_with("()") {
                    n
                } else {
                    String::new()
                }
            }
        };
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Shell builtins ([`SHELL_BUILTINS`]): no file effects besides redirections,
/// which the registry records for every program.
pub struct ShellBuiltin;

impl Recognizer for ShellBuiltin {
    fn id(&self) -> &'static str {
        "shell-builtin"
    }
    fn programs(&self) -> &'static [&'static str] {
        SHELL_BUILTINS
    }
    fn matches(&self, cmd: &Command) -> bool {
        // Builtins the `noop` recognizer already knows (`echo`, `exit`, ...) stay its
        // own.
        cmd.program().is_some_and(|p| SHELL_BUILTINS.contains(&p))
            && !crate::recognizers::fs::Ignore.matches(cmd)
    }
    fn recognize(&self, _: &Command, _: &RecognizeCx<'_>, _: &mut TaskSpec) -> Recognition {
        Recognition::Ignored
    }
}

/// Maximum nesting of scripts running or sourcing scripts.
const MAX_DEPTH: usize = 6;

/// Whether `p` names a shell script by its extension.
pub fn is_script_path(p: &str) -> bool {
    Path::new(p)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SCRIPT_EXTS.contains(&e))
}

/// The result of [`rewrite_anchors`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rewritten {
    /// The source with every anchor expression replaced by a same-length
    /// `${_TTD…}`.
    pub text: String,
    /// The variable names introduced (bind each to the script's directory).
    pub vars: Vec<String>,
}

impl Rewritten {
    /// `env` plus every anchor variable bound to `dir`.
    pub fn env_with(&self, env: &Env, dir: &str) -> Env {
        let mut e = env.clone();
        for v in &self.vars {
            e.insert(v.clone(), dir.to_string());
        }
        e
    }
}

/// Replaces script-directory anchor expressions with same-length variable
/// references. Linear: every candidate expression is scanned at most a bounded
/// distance.
pub fn rewrite_anchors(src: &str) -> Rewritten {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut vars: Vec<String> = Vec::new();
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        if b[i] != b'$' {
            i += 1;
            continue;
        }
        let end = match b.get(i + 1) {
            Some(b'(') => matching_paren(b, i + 1, 256),
            Some(b'{') => b[i + 2..]
                .iter()
                .take(64)
                .position(|&c| c == b'}')
                .map(|p| i + 2 + p),
            _ => None,
        };
        let Some(end) = end else {
            i += 1;
            continue;
        };
        let expr = &src[i..=end];
        let len = expr.len();
        if len >= 7 && is_anchor_expr(expr) {
            let name = format!(
                "{ANCHOR_PREFIX}{}",
                "_".repeat(len - 3 - ANCHOR_PREFIX.len())
            );
            out.push_str(&src[copied..i]);
            out.push_str("${");
            out.push_str(&name);
            out.push('}');
            if !vars.contains(&name) {
                vars.push(name);
            }
            i = end + 1;
            copied = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&src[copied..]);
    Rewritten { text: out, vars }
}

/// The index of the `)` matching the `(` at `open`, scanning at most `limit`
/// bytes and respecting quotes and nesting.
fn matching_paren(b: &[u8], open: usize, limit: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut j = open;
    let stop = b.len().min(open + limit);
    while j < stop {
        let c = b[j];
        match quote {
            Some(q) => {
                if c == b'\\' && q == b'"' {
                    j += 1;
                } else if c == q {
                    quote = None;
                } else if c == b'$' && q == b'"' && b.get(j + 1) == Some(&b'(') {
                    // `"$(…)"` inside double quotes nests.
                    let inner = matching_paren(b, j + 1, stop.saturating_sub(j))?;
                    j = inner;
                    quote = Some(b'"');
                }
            }
            None => match c {
                b'\\' => j += 1,
                b'\'' | b'"' => quote = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(j);
                    }
                }
                b'\n' => return None,
                _ => {}
            },
        }
        j += 1;
    }
    None
}

/// Whether `expr` (`$(…)` or `${…}`) evaluates to the running script's
/// directory.
pub fn is_anchor_expr(expr: &str) -> bool {
    is_anchor_expr_d(expr, 0)
}

fn is_anchor_expr_d(expr: &str, depth: usize) -> bool {
    if depth > 3 {
        return false;
    }
    if let Some(inner) = expr.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        return matches!(
            inner,
            "BASH_SOURCE%/*" | "BASH_SOURCE[0]%/*" | "0%/*" | "0:A:h" | "0:a:h" | "0:h"
        );
    }
    let Some(inner) = expr.strip_prefix("$(").and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let s = normalize_anchor_inner(inner, depth);
    matches!(
        s.as_str(),
        "dirname @0@" | "cd @D@ && pwd" | "cd @D@ ; pwd" | "cd @D@; pwd"
    )
}

/// Canonicalizes the inside of a `$(…)` for [`is_anchor_expr`]: script-path
/// expressions become `@0@`, nested anchors `@D@`; quotes, redirections and
/// `-P`/`--` are dropped.
fn normalize_anchor_inner(inner: &str, depth: usize) -> String {
    // Nested anchors first (`cd "$(dirname "$0")"`).
    let mut s = String::new();
    let b = inner.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        if b[i] == b'$'
            && let Some(end) = match b.get(i + 1) {
                Some(b'(') => matching_paren(b, i + 1, 256),
                Some(b'{') => b[i + 2..]
                    .iter()
                    .take(64)
                    .position(|&c| c == b'}')
                    .map(|p| i + 2 + p),
                _ => None,
            }
        {
            let e = &inner[i..=end];
            let rep = if is_anchor_expr_d(e, depth + 1) {
                "@D@"
            } else if is_script_path_expr(e) {
                "@0@"
            } else {
                // Any other substitution: an opaque token (scanned once, not re-entered).
                "@X@"
            };
            s.push_str(&inner[copied..i]);
            s.push_str(rep);
            i = end + 1;
            copied = i;
            continue;
        }
        i += 1;
    }
    s.push_str(&inner[copied..]);
    let mut s = s.replace("$0", "@0@").replace("$BASH_SOURCE", "@0@");
    for r in [
        "&>/dev/null",
        "&> /dev/null",
        ">/dev/null",
        "> /dev/null",
        "2>&1",
        "2>/dev/null",
        "2> /dev/null",
    ] {
        s = s.replace(r, " ");
    }
    s = s.replace(['"', '\''], "");
    let words: Vec<&str> = s
        .split_whitespace()
        .filter(|w| !matches!(*w, "-P" | "-L" | "--" | "builtin" | "command"))
        .collect();
    words.join(" ")
}

/// `${0}`, `${BASH_SOURCE[0]}`, `${BASH_SOURCE}`, `$(readlink -f "$0")`,
/// `$(realpath "$0")`.
fn is_script_path_expr(e: &str) -> bool {
    if let Some(inner) = e.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        return matches!(inner, "0" | "BASH_SOURCE" | "BASH_SOURCE[0]");
    }
    if let Some(inner) = e.strip_prefix("$(").and_then(|s| s.strip_suffix(')')) {
        let n = normalize_simple(inner);
        return matches!(
            n.as_str(),
            "readlink -f @0@" | "realpath @0@" | "greadlink -f @0@" | "readlink @0@"
        );
    }
    false
}

fn normalize_simple(inner: &str) -> String {
    let s = inner
        .replace("${BASH_SOURCE[0]}", "@0@")
        .replace("${BASH_SOURCE}", "@0@")
        .replace("${0}", "@0@")
        .replace("$BASH_SOURCE", "@0@")
        .replace("$0", "@0@")
        .replace(['"', '\''], "");
    s.split_whitespace()
        .filter(|w| *w != "--")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Converts possibly invalid UTF-8 to a `String` of the same byte length (each
/// invalid byte becomes `?`), so byte offsets stay valid. Whitespace other than
/// space, tab, CR and LF is blanked too ([`blank_odd_whitespace`]).
pub fn lossy_same_len(src: &[u8]) -> String {
    let mut s = String::with_capacity(src.len());
    for chunk in src.utf8_chunks() {
        s.push_str(chunk.valid());
        for _ in chunk.invalid() {
            s.push('?');
        }
    }
    blank_odd_whitespace(&s).into_owned()
}

/// Replaces whitespace characters other than space, tab, CR and LF (vertical
/// tab, form feed, NBSP, U+2028, ...) with spaces of the same UTF-8 length. The
/// task shell parser treats them as word ends without skipping them, which
/// never terminates; task sources apply this to their command lines.
pub fn blank_odd_whitespace(s: &str) -> std::borrow::Cow<'_, str> {
    let odd = |c: char| c.is_whitespace() && !matches!(c, ' ' | '\t' | '\r' | '\n');
    if !s.chars().any(odd) {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if odd(c) {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Applies [`blank_odd_whitespace`] to every command line of `tasks`.
pub fn sanitize_lines(tasks: &mut [crate::sources::DiscoveredTask]) {
    for t in tasks {
        for l in &mut t.lines {
            if let std::borrow::Cow::Owned(o) = blank_odd_whitespace(l) {
                *l = o;
            }
        }
    }
}

/// The command name and arguments of a shell invocation: `bash [opts] file
/// args…` → `Script(file, args)`, `bash -c 'cmds'` → `Inline(cmds)`, `bash`
/// alone → `Stdin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellInvocation<'a> {
    /// A script file and its arguments.
    Script(&'a Word, &'a [Word]),
    /// `-c` command string.
    Inline(&'a Word),
    /// Commands from standard input.
    Stdin,
}

/// Parses the arguments of `bash`/`sh`/... (`argv[0]` is the shell).
pub fn shell_invocation(cmd: &Command) -> ShellInvocation<'_> {
    let a = cmd.arg_words();
    let mut i = 0;
    while i < a.len() {
        let t = a[i].text.as_str();
        if t == "-c" {
            return match a.get(i + 1) {
                Some(w) => ShellInvocation::Inline(w),
                None => ShellInvocation::Stdin,
            };
        }
        if t == "--" {
            i += 1;
            break;
        }
        if t == "-o" || t == "+o" || t == "-O" || t == "+O" || t == "--rcfile" || t == "--init-file"
        {
            i += 2;
            continue;
        }
        if t.starts_with('-') || t.starts_with('+') {
            // `-euxo pipefail`: a combined flag ending in `o` takes a value.
            if t.len() > 2 && t.starts_with('-') && !t.starts_with("--") && t.ends_with('o') {
                i += 2;
            } else if t.len() > 1 && t.starts_with('-') && !t.starts_with("--") && t.contains('c') {
                return match a.get(i + 1) {
                    Some(w) => ShellInvocation::Inline(w),
                    None => ShellInvocation::Stdin,
                };
            } else {
                i += 1;
            }
            continue;
        }
        break;
    }
    match a.get(i) {
        Some(w) => ShellInvocation::Script(w, &a[i + 1..]),
        None => ShellInvocation::Stdin,
    }
}

thread_local! {
    /// Scripts being inlined on this thread (cycle and depth guard).
    static STACK: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    /// Shell functions defined by the scripts being inlined (and the files they sourced).
    static FUNCS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `bash x.sh`, `sh -c '…'`, `./scripts/x.sh`, `source x.sh` / `. x.sh` in task
/// commands: the script is an entry and its commands are inlined.
pub struct ShellScript;

impl Recognizer for ShellScript {
    fn id(&self) -> &'static str {
        "shell-script"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "bash", "sh", "zsh", "dash", "ksh", "ash", "mksh", "source", ".",
        ]
    }
    fn matches(&self, cmd: &Command) -> bool {
        let Some(p) = cmd.program() else {
            return false;
        };
        if p == "." || p == "source" {
            return true;
        }
        if p.contains('/') {
            return is_script_path(p);
        }
        cmd.program_name().is_some_and(|n| SHELLS.contains(&n))
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program().unwrap_or("").to_string();
        if prog == "." || prog == "source" {
            let Some(w) = cmd.argv.get(1) else {
                return Recognition::Ignored;
            };
            if w.dynamic {
                out.uncertain(
                    UncertaintyKind::ComputedSpecifier,
                    format!("`{prog}` of a computed path `{}`", w.text),
                );
                return Recognition::Partial;
            }
            let p = cmd.path(&w.text);
            return inline_script(&p, &w.text, cmd.argv.get(2..).unwrap_or(&[]), cmd, cx, out);
        }
        if prog.contains('/') {
            if cmd.argv.first().is_some_and(|w| w.dynamic || w.glob) {
                out.uncertain(
                    UncertaintyKind::ComputedSpecifier,
                    format!("runs a computed script `{prog}`"),
                );
                return Recognition::Partial;
            }
            let p = cmd.path(&prog);
            return inline_script(&p, &prog, cmd.arg_words(), cmd, cx, out);
        }
        let shell_name = cmd.program_name().unwrap_or("sh").to_string();
        out.tool("system", shell_name.as_str());
        match shell_invocation(cmd) {
            ShellInvocation::Script(w, args) => {
                if w.dynamic {
                    out.uncertain(
                        UncertaintyKind::ComputedSpecifier,
                        format!("`{shell_name}` runs a computed script `{}`", w.text),
                    );
                    return Recognition::Partial;
                }
                let p = cmd.path(&w.text);
                inline_script(&p, &w.text, args, cmd, cx, out)
            }
            ShellInvocation::Inline(w) => {
                if w.dynamic {
                    out.uncertain(
                        UncertaintyKind::ShellUnsupported,
                        format!("`{shell_name} -c` with a computed command"),
                    );
                    return Recognition::Partial;
                }
                if STACK.with(|s| s.borrow().len()) >= MAX_DEPTH {
                    out.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("`{shell_name} -c` nested too deeply"),
                    );
                    return Recognition::Partial;
                }
                let parsed = shell::parse(&blank_odd_whitespace(&w.text), &cmd.cwd, cx.env);
                out.uncertainties
                    .extend(parsed.uncertainties.iter().cloned());
                STACK.with(|s| s.borrow_mut().push(PathBuf::from("<-c>")));
                let mut r = if parsed.uncertainties.is_empty() {
                    Recognition::Full
                } else {
                    Recognition::Partial
                };
                for c in &parsed.commands {
                    r = worse(r, registry().recognize(c, cx, out).recognition);
                }
                STACK.with(|s| {
                    s.borrow_mut().pop();
                });
                r
            }
            ShellInvocation::Stdin => {
                if cmd
                    .redirects
                    .iter()
                    .any(|r| r.is_input() && !r.target.dynamic)
                {
                    // `bash < script.sh`: the redirect is an input; its commands are unknown.
                    out.uncertain(
                        UncertaintyKind::UnknownCommand,
                        format!("`{shell_name}` reading a script from a redirect"),
                    );
                    return Recognition::Partial;
                }
                Recognition::Ignored
            }
        }
    }
}

/// Inlines the script at `path` (as written: `shown`) run with `args` by `cmd`.
fn inline_script(
    path: &Path,
    shown: &str,
    args: &[Word],
    cmd: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    out.entry(path.to_path_buf(), EntryLang::Auto);
    // The script (and every file it sources) defines the task ([`crate::runs`]).
    out.defines(path.to_path_buf());
    let (depth, cycle) = STACK.with(|s| {
        let s = s.borrow();
        (s.len(), s.iter().any(|p| p == path))
    });
    if cycle {
        return Recognition::Full;
    }
    if depth >= MAX_DEPTH {
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("script `{shown}` nested too deeply"),
        );
        return Recognition::Partial;
    }
    let Ok(src) = cx.read_to_string(path) else {
        // Missing (perhaps generated by an earlier task) or unreadable: contents
        // unknown.
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("script `{shown}` not readable"),
        );
        return Recognition::Partial;
    };
    if let Some(interp) = shebang_interpreter(&src)
        && !SHELLS.contains(&interp.as_str())
    {
        // `./scripts/gen.sh` with a `#!/usr/bin/env node` shebang.
        return match interp.as_str() {
            "node" | "tsx" | "ts-node" | "bun" => {
                out.entries.retain(|e| e.path != path);
                out.entry(path.to_path_buf(), EntryLang::Js);
                Recognition::Full
            }
            "python" | "python3" => {
                out.entries.retain(|e| e.path != path);
                out.entry(path.to_path_buf(), EntryLang::Python);
                Recognition::Full
            }
            other => {
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("script `{shown}` run by `{other}`"),
                );
                Recognition::Partial
            }
        };
    }
    let rw = rewrite_anchors(&blank_odd_whitespace(&src));
    let dir = path
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut env = rw.env_with(cx.env, &dir);
    env.insert("0".into(), shown.to_string());
    env.insert("BASH_SOURCE".into(), path.to_string_lossy().into_owned());
    for (i, a) in args.iter().enumerate().take(9) {
        if a.is_static() {
            env.insert((i + 1).to_string(), a.text.clone());
        }
    }
    let parsed = shell::parse(&rw.text, &cmd.cwd, &env);
    if !parsed.uncertainties.is_empty() {
        // Command substitutions and `eval` may start tests ([`crate::runs`]).
        crate::runs::scan_unsupported_shell(&rw.text, cx.repo_root, out);
    }
    let mut r = Recognition::Full;
    for u in &parsed.uncertainties {
        if u.detail.contains("of a shell file") {
            continue; // `source` is followed below.
        }
        out.uncertainties.push(u.clone());
        r = Recognition::Partial;
    }
    STACK.with(|s| s.borrow_mut().push(path.to_path_buf()));
    FUNCS.with(|f| f.borrow_mut().extend(functions(&rw.text)));
    let cx2 = RecognizeCx { env: &env, ..*cx };
    for c in &parsed.commands {
        let prog = c.program().unwrap_or("");
        let is_func = FUNCS.with(|f| f.borrow().iter().any(|n| n == prog));
        if SHELL_BUILTINS.contains(&prog) || is_func {
            // A builtin or a function defined by the scripts (its body's commands are
            // recognized where it is defined): only its redirections matter.
            for rd in &c.redirects {
                if rd.is_null() || rd.target.dynamic {
                    continue;
                }
                let p = c.path(&rd.target.text);
                if rd.is_input() {
                    out.input(p);
                } else if rd.op != "<<<" {
                    out.write(p, cx.repo_root);
                }
            }
            continue;
        }
        r = worse(r, registry().recognize(c, &cx2, out).recognition);
    }
    let outermost = STACK.with(|s| {
        let mut s = s.borrow_mut();
        s.pop();
        s.is_empty()
    });
    if outermost {
        FUNCS.with(|f| f.borrow_mut().clear());
    }
    r
}

/// The interpreter named by a `#!` line (`#!/usr/bin/env -S node --x` → `node`,
/// `#!/bin/bash -e` → `bash`), without a version suffix (`python3.12` →
/// `python3`).
pub fn shebang_interpreter(src: &str) -> Option<String> {
    let line = src.strip_prefix("#!")?.lines().next()?.trim();
    let mut words = line.split_whitespace();
    let first = words.next()?;
    let mut prog = first.rsplit('/').next().unwrap_or(first);
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
        prog = prog.rsplit('/').next().unwrap_or(prog);
    }
    let base = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let name = match base {
        "python" if prog.starts_with("python3") => "python3",
        b => b,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_keep_length() {
        for src in [
            r#"source "$(dirname "$0")/lib.sh""#,
            r#"DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; . "$DIR/x.sh""#,
            r#"DIR="$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )""#,
            ". ${BASH_SOURCE%/*}/a.sh",
            "source ${0:A:h}/b.zsh",
            r#"x=$(dirname "$(readlink -f "$0")")"#,
        ] {
            let r = rewrite_anchors(src);
            assert_eq!(r.text.len(), src.len(), "{src}");
            assert!(!r.vars.is_empty(), "{src} -> {}", r.text);
        }
        let r = rewrite_anchors(r#"x=$(dirname "$FOO")"#);
        assert!(r.vars.is_empty());
        let r = rewrite_anchors("echo $(pwd)");
        assert!(r.vars.is_empty());
    }

    #[test]
    fn anchors_never_panic() {
        let seeds = [
            "$(",
            "${",
            "$((",
            "$(\"",
            "$(dirname \"$0",
            "${BASH_SOURCE%/*",
            "é$(é)",
            "$($($(",
        ];
        for s in seeds {
            for n in 0..s.len() {
                if s.is_char_boundary(n) {
                    rewrite_anchors(&s[..n]);
                }
            }
        }
        let mut x: u64 = 0x9e3779b97f4a7c15;
        for _ in 0..2000 {
            let mut v = Vec::new();
            for _ in 0..40 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let a = b"$(){}\"' \\dirname0BASH_SOURCE%/*cdpwd&\n";
                v.push(a[(x % a.len() as u64) as usize]);
            }
            let s = lossy_same_len(&v);
            let r = rewrite_anchors(&s);
            assert_eq!(r.text.len(), s.len());
        }
    }

    #[test]
    fn builtins_and_functions() {
        let root = std::path::Path::new("/repo");
        let env = std::collections::BTreeMap::new();
        let cx = RecognizeCx {
            repo_root: root,
            package_root: root,
            env: &env,
            task_name: None,
            scripts: &[],
        };
        let parsed = crate::shell::parse("local x=1; read -r y < in.txt; shift", root, &env);
        let mut out = TaskSpec::default();
        for c in &parsed.commands {
            let r = crate::registry::registry().recognize(c, &cx, &mut out);
            assert_eq!(r.recognizer, Some("shell-builtin"), "{}", c.text);
            assert_eq!(r.recognition, Recognition::Ignored);
        }
        assert_eq!(out.inputs, [root.join("in.txt")]);
        assert!(out.uncertainties.is_empty() && out.memberships.is_empty());
        let f =
            functions("emit() {\n  echo hi\n}\nfunction log_it {\n:\n}\nx () { :; }\necho a() b");
        assert_eq!(f, ["emit", "log_it", "x"]);
    }

    #[test]
    fn shebangs() {
        assert_eq!(
            shebang_interpreter("#!/usr/bin/env node\n").as_deref(),
            Some("node")
        );
        assert_eq!(
            shebang_interpreter("#!/bin/bash -e\n").as_deref(),
            Some("bash")
        );
        assert_eq!(
            shebang_interpreter("#!/usr/bin/env -S python3.12 -u\n").as_deref(),
            Some("python3")
        );
        assert_eq!(shebang_interpreter("echo"), None);
    }

    #[test]
    fn invocations() {
        let p = |s: &str| shell::parse(s, Path::new("/r"), &Env::new()).commands[0].clone();
        let c = p("bash -euo pipefail x.sh a");
        assert!(
            matches!(shell_invocation(&c), ShellInvocation::Script(w, a) if w.text == "x.sh" && a.len() == 1)
        );
        let c = p("sh -c 'node a.js'");
        assert!(
            matches!(shell_invocation(&c), ShellInvocation::Inline(w) if w.text == "node a.js")
        );
        let c = p("bash");
        assert_eq!(shell_invocation(&c), ShellInvocation::Stdin);
    }
}
