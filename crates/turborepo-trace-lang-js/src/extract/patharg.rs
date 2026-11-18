//! Path arguments: a file path handed to code this file imports.
//!
//! ```ts
//! import { loadConfig } from 'some-package/testing';
//! const config = loadConfig('./routes.jsonc');
//! ```
//!
//! The read happens inside `some-package`, which query mode never opens, so no
//! reference shows that this file depends on `routes.jsonc`: the path literal
//! is the only trace. A real test compared such a config with a list in code,
//! and a change of the config alone selected nothing.
//!
//! The rule: a string literal that is written as a relative file path (it
//! starts with `./` or `../` and its last segment has an extension) and is
//! passed directly as an argument of a call whose callee is an imported binding
//! (`f('./x.json')`, `ns.f('./x.json')`, `new C('./x.json')`, with `import` or
//! `require`) may be a file the callee reads. It is emitted as an optional
//! [`RefKind::Asset`], once relative to the working directory (what
//! file APIs do with a relative path) and once relative to the file's directory
//! (what loaders do that resolve against their caller). A JavaScript or
//! TypeScript source file is only taken relative to the file
//! (`mock.module('../lib/x.ts')`): it is named the way an import names it, and
//! a working-directory guess would leave a record on every test whose working
//! directory is not known. Assets only ever add inputs: a path that does not
//! exist is a probe.
//!
//! Not covered, on purpose: bindings of the modules whose path arguments the
//! main pass already reads (`fs`, `path`, `url`, `module`, `child_process`,
//! `worker_threads`), test doubles (`jest.mock('./x.js')`, handled by the
//! framework adapters), calls on values that are not imports
//! (`expect(x).toBe('./a.js')`, `Promise.resolve('./a.js')`), and strings
//! that are not written as a relative file (`'settings'`, `'/api/x.json'`,
//! URLs): those are names, not paths, and belong to the conservative string
//! scan.

use std::collections::HashMap;

use turborepo_trace_core::{RefFlags, RefKind, Span, eval::Value};

use super::{
    Recognizer, Site,
    api::BaseRule,
    deep::{NONE, Tree},
    evaluator::{self, Anchor, JsVal, Outcome},
};
use crate::lexer::{Kw, TokKind, Token};

/// Maximum length of a path argument.
const MAX_LEN: usize = 200;
/// Maximum path arguments emitted per file.
const MAX_PER_FILE: usize = 256;

/// Whether `s` is written as a relative file path: `./` or `../` first, name
/// characters only, and an extension on the last segment.
pub(super) fn shape(s: &[u8]) -> bool {
    if s.len() < 5 || s.len() > MAX_LEN || !(s.starts_with(b"./") || s.starts_with(b"../")) {
        return false;
    }
    if !s.iter().all(|&c| {
        c.is_ascii_alphanumeric() || matches!(c, b'/' | b'.' | b'-' | b'_' | b'@' | b'+' | b'~')
    }) {
        return false;
    }
    let last = s.rsplit(|&c| c == b'/').next().unwrap_or(s);
    match last.iter().rposition(|&c| c == b'.') {
        // `name.ext`: a stem, and a short alphanumeric extension.
        Some(dot) if dot > 0 => {
            let ext = &last[dot + 1..];
            (1..=8).contains(&ext.len()) && ext.iter().all(u8::is_ascii_alphanumeric)
        }
        _ => false,
    }
}

/// Whether `src` may contain a path argument: a quote followed by `./` or `../`
/// right after `(` or `,` (a cheap scan; the pass then checks tokens).
pub(super) fn may_have(src: &[u8]) -> bool {
    for q in memchr::memchr3_iter(b'\'', b'"', b'`', src) {
        if !(src[q + 1..].starts_with(b"./") || src[q + 1..].starts_with(b"../")) {
            continue;
        }
        let mut i = q;
        while i > 0 && src[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        if i > 0 && matches!(src[i - 1], b'(' | b',') {
            return true;
        }
    }
    false
}

/// Whether `spec` names a JavaScript or TypeScript source file.
fn module_file(spec: &str) -> bool {
    matches!(
        spec.rsplit('.').next(),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts")
    )
}

/// Modules whose path arguments the main pass reads itself.
fn tracked_module(m: &str) -> bool {
    let m = m.strip_prefix("node:").unwrap_or(m);
    matches!(
        m,
        "fs" | "fs/promises"
            | "fs-extra"
            | "graceful-fs"
            | "path"
            | "path/posix"
            | "url"
            | "module"
            | "child_process"
            | "worker_threads"
            | "vitest"
            | "@jest/globals"
    )
}

impl<'s> Recognizer<'s, '_> {
    /// The names this file binds by `import` or `require`, with their module.
    fn import_bindings(&self, toks: &[Token]) -> HashMap<&'s [u8], &'s [u8]> {
        let src = self.src;
        let mut out: HashMap<&'s [u8], &'s [u8]> = HashMap::new();
        let binding = |t: &Token| t.kind == TokKind::Ident && t.kw == Kw::None;
        let mut k = 0;
        while k < toks.len() {
            let t = &toks[k];
            // `import a, { b, c as d }, * as ns from 'm'`
            if t.kw == Kw::Import && t.depth == 0 {
                let mut names: Vec<&'s [u8]> = Vec::new();
                let mut j = k + 1;
                while j < toks.len() && j < k + 512 {
                    let x = &toks[j];
                    if x.kind == TokKind::Str {
                        if let Some(m) = x.literal_content(src) {
                            for n in names.drain(..) {
                                out.insert(n, m);
                            }
                        }
                        break;
                    }
                    if matches!(x.kind, TokKind::LParen | TokKind::Semi | TokKind::Assign) {
                        break;
                    }
                    // A local name is what stands before `,`, `}` or `from`.
                    if binding(x)
                        && toks.get(j + 1).is_some_and(|n| {
                            matches!(n.kind, TokKind::Comma | TokKind::RBrace)
                                || n.text(src) == b"from"
                        })
                        && x.text(src) != b"type"
                    {
                        names.push(x.text(src));
                    }
                    j += 1;
                }
                k = j + 1;
                continue;
            }
            // `const a = require('m')`, `const { b, c: d } = require('m')`
            if t.kw == Kw::Require
                && k >= 2
                && toks[k - 1].kind == TokKind::Assign
                && toks.get(k + 1).is_some_and(|n| n.kind == TokKind::LParen)
                && toks.get(k + 2).is_some_and(|n| n.kind == TokKind::Str)
                && let Some(m) = toks[k + 2].literal_content(src)
            {
                let lhs = &toks[k - 2];
                if binding(lhs) {
                    out.insert(lhs.text(src), m);
                } else if lhs.kind == TokKind::RBrace {
                    let mut j = k - 2;
                    while j > 0 && toks[j].kind != TokKind::LBrace && k - j < 256 {
                        let x = &toks[j];
                        if binding(x)
                            && matches!(toks[j + 1].kind, TokKind::Comma | TokKind::RBrace)
                        {
                            out.insert(x.text(src), m);
                        }
                        j -= 1;
                    }
                }
            }
            k += 1;
        }
        out
    }

    /// Emits the path arguments of the file (see the module docs).
    pub(super) fn deep_path_args(&mut self, toks: &[Token], tree: &Tree) {
        let src = self.src;
        let imports = self.import_bindings(toks);
        if imports.is_empty() {
            return;
        }
        let covered: Vec<Span> = self.ex.refs.iter().map(|r| r.span).collect();
        let mut sites: Vec<(Span, &'s str)> = Vec::new();
        for k in 2..toks.len().saturating_sub(1) {
            let t = &toks[k];
            if !matches!(t.kind, TokKind::Str | TokKind::Template) {
                continue;
            }
            // Directly an argument: `( 'x' ,` / `, 'x' )`.
            let p = tree.parent[k];
            if p == NONE
                || toks[p as usize].kind != TokKind::LParen
                || !matches!(toks[k - 1].kind, TokKind::LParen | TokKind::Comma)
                || !matches!(toks[k + 1].kind, TokKind::Comma | TokKind::RParen)
            {
                continue;
            }
            let Some(content) = t.literal_content(src) else {
                continue;
            };
            if !shape(content) || covered.iter().any(|c| c.start <= t.start && t.end <= c.end) {
                continue;
            }
            // The callee: `root`, `root.a.b`, with `root` an import of an untracked module.
            let p = p as usize;
            if p == 0 || toks[p - 1].kind != TokKind::Ident || toks[p - 1].kw != Kw::None {
                continue;
            }
            let mut r = p - 1;
            while r >= 2
                && matches!(toks[r - 1].kind, TokKind::Dot | TokKind::OptChain)
                && toks[r - 2].kind == TokKind::Ident
                && toks[r - 2].kw == Kw::None
            {
                r -= 2;
            }
            if r >= 1 && matches!(toks[r - 1].kind, TokKind::Dot | TokKind::OptChain) {
                // A member of something that is not a plain name (a call result, `this`).
                continue;
            }
            let Some(module) = imports.get(toks[r].text(src)) else {
                continue;
            };
            if std::str::from_utf8(module).is_ok_and(tracked_module) {
                continue;
            }
            let Ok(spec) = std::str::from_utf8(content) else {
                continue;
            };
            sites.push((t.span(), spec));
            if sites.len() >= MAX_PER_FILE {
                break;
            }
        }
        for (span, spec) in sites {
            let detail = format!("path argument {spec}");
            // A source file is named the way an import names it: against the caller
            // (`mock.module('../lib/x.ts')`, a worker or loader registration). Only a data
            // file is also read the way file APIs read: against the working directory.
            let mut alternatives = vec![JsVal::Path(Anchor::FileDir, Value::Str(spec.to_owned()))];
            if !module_file(spec) {
                alternatives.insert(0, JsVal::Path(Anchor::Cwd, Value::Str(spec.to_owned())));
            }
            self.emit_outcome(
                Site {
                    kind: RefKind::Asset,
                    rule: BaseRule::Path,
                    span,
                    arg_span: span,
                    flags: RefFlags::OPTIONAL,
                    requires: false,
                },
                Outcome {
                    val: evaluator::alts(alternatives),
                    env: false,
                    complete: true,
                    guess: None,
                    named: false,
                    flow: false,
                },
                &detail,
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use turborepo_trace_core::{Base, RefKind};

    use crate::{
        extract::{ScanOptions, scan_named},
        lexer::Dialect,
    };

    /// The asset references of `src`: `<base>:<spec>` (with ` !<uncertainty
    /// kind>`).
    fn assets(src: &str) -> Vec<String> {
        let ex = scan_named(
            src.as_bytes(),
            Dialect::TS,
            ScanOptions::default(),
            Some("main.test.ts"),
        );
        ex.refs
            .iter()
            .filter(|r| r.kind == RefKind::Asset)
            .map(|r| {
                let base = match r.base {
                    Base::FileDir => "dir",
                    Base::Cwd => "cwd",
                    _ => "other",
                };
                let mut s = format!("{base}:{}", r.spec);
                if let Some(u) = &r.uncertainty {
                    s.push_str(&format!(" !{}", u.kind.as_str()));
                }
                s
            })
            .collect()
    }

    #[test]
    fn shapes() {
        for ok in [
            "./routes.jsonc",
            "../data/x.json",
            "./a/b-c/d_e.test.ts",
            "./.env.test",
        ] {
            assert!(super::shape(ok.as_bytes()), "{ok}");
        }
        for no in [
            "./settings",
            "routes.jsonc",
            "/api/x.json",
            "https://x.example/a.json",
            "./a b.json",
            "./*.json",
            "./x.",
            "./",
            "../..",
            "./dir/",
        ] {
            assert!(!super::shape(no.as_bytes()), "{no}");
        }
    }

    /// The real shape: a config loader of a package, given a path relative to
    /// the working directory of the test.
    #[test]
    fn a_path_handed_to_an_imported_function() {
        let src = "import { loadConfigForEdge } from '@scope/routing/next/testing';\nimport { \
                   SLUGS } from './slugs.mjs';\nit('covers every route', () => {\n  const config \
                   = loadConfigForEdge('./routes.jsonc');\n  expect(config.paths.filter((p) => \
                   !SLUGS.has(p))).toEqual([]);\n});\n";
        let got = assets(src);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            got.iter()
                .any(|a| a.starts_with("cwd:") && a.contains("routes.jsonc")),
            "{got:?}"
        );
        assert!(
            got.iter()
                .any(|a| a.starts_with("dir:") && a.contains("routes.jsonc")),
            "{got:?}"
        );
    }

    #[test]
    fn callee_forms() {
        // A namespace member, a default import, `new`, `require`, a later argument, a
        // helper of the repository.
        for call in [
            "import * as ns from 'pkg';\nns.load('./a.json');\n",
            "import pkg from 'pkg';\npkg.sub.load('./a.json');\n",
            "import { Loader } from 'pkg';\nnew Loader('./a.json');\n",
            "const { load } = require('pkg');\nload('./a.json');\n",
            "const pkg = require('pkg');\npkg.load({ strict: true }, './a.json');\n",
            "import { load } from '../helpers/load';\nload('./a.json');\n",
        ] {
            assert_eq!(assets(call).len(), 2, "{call}: {:?}", assets(call));
        }
    }

    /// A source file is named like an import: relative to the file only, with
    /// no record.
    #[test]
    fn a_source_file_is_relative_to_the_caller() {
        let got = assets(
            "import { mock } from 'node:test';\nmock.module('../../lib/check.ts', { namedExports: \
             {} });\n",
        );
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(
            got[0].starts_with("dir:") && !got[0].contains('!'),
            "{got:?}"
        );
    }

    #[test]
    fn what_is_not_a_path_argument() {
        for src in [
            // Not an import.
            "function load(p: string) { return p; }\nload('./a.json');\n",
            "expect(x).toBe('./a.json');\nPromise.resolve('./a.json');\n",
            // A call result in the callee.
            "import { make } from 'pkg';\nmake().load('./a.json');\n",
            // Not written as a relative file.
            "import { go } from 'pkg';\ngo('./settings');\ngo('a.json');\ngo('/x/a.json');\n",
            // Not directly an argument.
            "import { load } from 'pkg';\nload(['./a.json']);\nload({ file: './a.json' \
             });\nload('./a.json' + x);\n",
            // Modules whose path arguments are read by the main pass or the adapters.
            "import { vi } from 'vitest';\nvi.mock('./a.js');\n",
            "import fs from 'node:fs';\nfs.readFileSync('./a.json');\n",
        ] {
            assert!(assets(src).is_empty(), "{src}: {:?}", assets(src));
        }
    }
}
