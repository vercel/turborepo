//! Shader modules (`.wgsl`) written with import lines.
//!
//! WGSL has no modules. Loaders add them: a JS/TS module imports a `.wgsl`
//! file, and the loader that compiles it (`@vgpu/wgsl`: a vite plugin, a
//! bundler loader) inlines the shaders that the file names in lines of ES
//! module syntax:
//!
//! ```text
//! import { hash21 } from "@vgpu/wgsl-std/hash";
//! import {
//!   Scene,
//!   project,
//! } from "./scene.wgsl";
//!
//! export fn shade(p: vec2f) -> vec3f { ... }
//! ```
//!
//! This plugin makes a `.wgsl` file a module of the graph: each import line is
//! a reference ([`RefKind::Import`]; `export … from` is a
//! [`RefKind::ReExport`]), resolved by the JS resolver like the import of a JS
//! module (a relative file as written, a package through `node_modules` or, in
//! the graph store, through the lockfile). So a changed shader reaches the
//! modules that import it through other shaders, and nothing the loader reads
//! for a module is left untraced.
//!
//! # What is read
//!
//! WGSL reserves the words `import` and `export` and has no string literals, so
//! a statement that starts with one of them belongs to a loader. Read are,
//! outside comments (`//`, nested `/* */`), at the start of a statement:
//!
//! | Statement | Reference |
//! |---|---|
//! | `import { a, b as c } from "m";` (also over several lines), `import * as n from "m";`, `import n from "m";` | `import m` |
//! | `import "m";` | `import m` |
//! | `export { a } from "m";`, `export * from "m";` | `re_export m` |
//! | `export fn …`, `export struct …`, `export const …`, `export { a };` | none: a declaration |
//!
//! The quotes may be single or double; the semicolon is optional.
//!
//! An `import` statement of any other form (`import package::util::hash;`, a
//! string that does not end on its line) is not understood, and neither is a
//! preprocessor line that includes a file (`#import …`, `#include …`): the file
//! gets a [`UncertaintyKind::ParseIncomplete`] record, which keeps the entries
//! that reach the file uncertain. An import that does not resolve keeps the
//! resolver's record, which is open in a file of this language.
//!
//! # What is assumed
//!
//! That the import lines are all a loader reads for a module, and that it
//! resolves them as a bundler resolves the import of a JS module: a relative
//! specifier names the file as written, anything else a package. A loader with
//! a module path of its own (an include directory, a registry of names) is not
//! modelled; its lines are of the forms that are recorded as not understood.

use turborepo_trace_core::{
    ExtractCx, FileId, LangId, Language, ProjectCx, RawRef, RefKind, RefSink, Resolution,
    ResolveCx, Resolver, Span, Specifier, Uncertainty, UncertaintyKind,
};

use crate::resolve::{JsResolveOptions, JsResolver};

/// Shader modules with import lines. See the module docs.
#[derive(Copy, Clone, Debug, Default)]
pub struct WgslLanguage;

impl Language for WgslLanguage {
    fn id(&self) -> LangId {
        LangId("wgsl")
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["wgsl"]
    }

    fn extract<'s>(&self, src: &'s [u8], _cx: &ExtractCx<'_>, out: &mut RefSink<'s>) {
        scan(src, out);
    }

    fn resolver(&self, project: &ProjectCx<'_>) -> Box<dyn Resolver> {
        Box::new(WgslResolver {
            js: JsResolver::new(project, &JsResolveOptions::default()),
        })
    }
}

/// Resolves a shader's imports as the imports of a JS module. Not the JS
/// resolver itself: a shader has no file-level edges (the package scope of a JS
/// file decides its module format; a shader has none).
struct WgslResolver {
    js: JsResolver,
}

impl Resolver for WgslResolver {
    fn resolve(&self, r: &RawRef<'_>, from: FileId, cx: &ResolveCx<'_>) -> Resolution {
        self.js.resolve(r, from, cx)
    }

    fn clear_cache(&self) {
        self.js.clear_cache();
    }
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The end of the comment that starts at `i` (`//` to the end of the line, `/*
/// */` with nesting), or `None` when no comment starts there.
fn comment_end(src: &[u8], i: usize) -> Option<usize> {
    match (src.get(i), src.get(i + 1)) {
        (Some(b'/'), Some(b'/')) => {
            Some(memchr::memchr(b'\n', &src[i..]).map_or(src.len(), |n| i + n))
        }
        (Some(b'/'), Some(b'*')) => {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < src.len() && depth > 0 {
                match (src[j], src.get(j + 1)) {
                    (b'/', Some(b'*')) => {
                        depth += 1;
                        j += 2;
                    }
                    (b'*', Some(b'/')) => {
                        depth -= 1;
                        j += 2;
                    }
                    _ => j += 1,
                }
            }
            Some(j)
        }
        _ => None,
    }
}

/// Skips whitespace (line breaks too) and comments from `i`.
fn skip_trivia(src: &[u8], mut i: usize) -> usize {
    loop {
        match src.get(i) {
            Some(b' ' | b'\t' | b'\r' | b'\n') => i += 1,
            Some(b'/') => match comment_end(src, i) {
                Some(e) => i = e,
                None => return i,
            },
            _ => return i,
        }
    }
}

/// How a statement that starts with `import` or `export` ended.
enum Stmt<'s> {
    /// It names a module: the specifier and the offset after the closing quote.
    Module(&'s str, usize),
    /// It names none (a declaration, an export list); scanning continues at the
    /// offset.
    Plain(usize),
    /// Not understood; scanning continues at the offset.
    Unreadable(usize),
}

/// Reads the statement whose keyword ends at `after`: the binding clause
/// (names, braces, commas, `*`), then `from` and a string. `bare`: a string may
/// follow the keyword directly (`import "m"`).
fn statement(src: &[u8], after: usize, bare: bool) -> Stmt<'_> {
    let mut j = after;
    // The last word before the current position, and whether anything was read.
    let mut last: &[u8] = b"";
    let mut any = false;
    loop {
        j = skip_trivia(src, j);
        let Some(&b) = src.get(j) else {
            return Stmt::Plain(j);
        };
        match b {
            b'"' | b'\'' => {
                if !(last == b"from" || (bare && !any)) {
                    return Stmt::Unreadable(j + 1);
                }
                let rest = &src[j + 1..];
                let line = memchr::memchr(b'\n', rest).unwrap_or(rest.len());
                let Some(n) = memchr::memchr(b, &rest[..line]) else {
                    return Stmt::Unreadable(j + 1 + line);
                };
                return match std::str::from_utf8(&rest[..n]) {
                    Ok(s) if !s.is_empty() => Stmt::Module(s, j + 1 + n + 1),
                    _ => Stmt::Unreadable(j + 1 + n + 1),
                };
            }
            b'{' | b'}' | b',' | b'*' => {
                last = b"";
                any = true;
                j += 1;
            }
            _ if is_ident(b) => {
                let s = j;
                while j < src.len() && is_ident(src[j]) {
                    j += 1;
                }
                last = &src[s..j];
                // The next statement (no semicolon in between).
                if matches!(last, b"import" | b"export") {
                    return Stmt::Plain(s);
                }
                any = true;
            }
            // The statement ends, or goes on with something a binding clause never has
            // (`::`, `(`, `<`): no module is named.
            _ => return Stmt::Plain(j),
        }
    }
}

/// Finds the import lines of a shader module. Linear; never panics.
pub fn scan<'s>(src: &'s [u8], out: &mut RefSink<'s>) {
    let mut i = 0;
    // Only trivia since the start of the file or the end of a statement or line.
    let mut start = true;
    while i < src.len() {
        let b = src[i];
        if let Some(e) = comment_end(src, i) {
            i = e;
            continue;
        }
        match b {
            b'\n' | b';' | b'}' => {
                start = true;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            // A preprocessor line. WGSL has none, so it is some loader's; one that
            // includes a file is an import this plugin cannot read.
            b'#' if start => {
                let end = memchr::memchr(b'\n', &src[i..]).map_or(src.len(), |n| i + n);
                let word = src[i + 1..end]
                    .iter()
                    .position(|c| !is_ident(*c))
                    .map_or(end, |n| i + 1 + n);
                if matches!(&src[i + 1..word], b"import" | b"include") {
                    out.diagnostic(Uncertainty::new(
                        UncertaintyKind::ParseIncomplete,
                        Span::new(i as u32, end as u32),
                        "a preprocessor line that includes another file: what it loads is not \
                         traced",
                    ));
                }
                i = end;
            }
            _ if is_ident(b) => {
                let s = i;
                while i < src.len() && is_ident(src[i]) {
                    i += 1;
                }
                let word = &src[s..i];
                let import = word == b"import";
                if !(start && (import || word == b"export")) {
                    start = false;
                    continue;
                }
                start = false;
                // `export fn`, `export struct`, ...: a declaration, read on as code.
                if !import && !matches!(src.get(skip_trivia(src, i)), Some(b'{' | b'*')) {
                    continue;
                }
                match statement(src, i, import) {
                    Stmt::Module(spec, end) => {
                        let kind = if import {
                            RefKind::Import
                        } else {
                            RefKind::ReExport
                        };
                        out.push(RawRef::new(
                            kind,
                            Specifier::literal(spec),
                            Span::new(s as u32, end as u32),
                        ));
                        i = end;
                    }
                    // `export { a };`: a list of the file's own declarations.
                    Stmt::Plain(end) if !import => i = end,
                    Stmt::Plain(end) | Stmt::Unreadable(end) => {
                        let end = end.max(i);
                        out.diagnostic(Uncertainty::new(
                            UncertaintyKind::ParseIncomplete,
                            Span::new(s as u32, end as u32),
                            "an import statement that does not name its module as `from \
                             \"<module>\"`: what it loads is not traced",
                        ));
                        i = end;
                    }
                }
                // A statement ended here, with or without a semicolon.
                start = true;
            }
            _ => {
                start = false;
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `kind spec` per reference, then `! <text of the span>` per diagnostic.
    fn refs(src: &str) -> Vec<String> {
        let mut sink = RefSink::new();
        scan(src.as_bytes(), &mut sink);
        let (refs, diags) = sink.into_parts();
        let mut out: Vec<String> = refs
            .iter()
            .map(|r| format!("{} {}", r.kind, r.spec))
            .collect();
        out.extend(diags.iter().map(|d| format!("! {}", &src[d.span.range()])));
        out
    }

    #[test]
    fn import_lines() {
        assert_eq!(
            refs("import { a, b as c } from \"./x.wgsl\";\nfn f() -> f32 { return a(); }\n"),
            ["import ./x.wgsl"]
        );
        // Several lines, single quotes, no semicolon, a package.
        assert_eq!(
            refs(
                "import {\n  Scene,\n  project, // the projection\n} from './scene.wgsl'\nimport \
                 { hash21 } from \"@vgpu/wgsl-std/hash\";\nimport * as noise from \
                 \"../noise.wgsl\";\nimport util from \"./util.wgsl\";\nimport \"./side.wgsl\";\n"
            ),
            [
                "import ./scene.wgsl",
                "import @vgpu/wgsl-std/hash",
                "import ../noise.wgsl",
                "import ./util.wgsl",
                "import ./side.wgsl",
            ]
        );
        // Indented, after a statement on the same line, after a block.
        assert_eq!(
            refs(
                "  import { a } from \"./a.wgsl\"; import { b } from \"./b.wgsl\";\nstruct S { x: \
                 f32 }\nimport { c } from \"./c.wgsl\";"
            ),
            ["import ./a.wgsl", "import ./b.wgsl", "import ./c.wgsl"]
        );
    }

    #[test]
    fn spans_cover_the_statement() {
        let src = "// head\nimport { a } from \"./a.wgsl\";\n";
        let mut sink = RefSink::new();
        scan(src.as_bytes(), &mut sink);
        let (refs, _) = sink.into_parts();
        assert_eq!(&src[refs[0].span.range()], "import { a } from \"./a.wgsl\"");
    }

    #[test]
    fn exports() {
        // Declarations and export lists name no module.
        assert_eq!(
            refs(
                "export fn shade(p: vec2f) -> vec3f { return vec3f(p, 0.0); }\nexport struct \
                 Scene { time: f32 }\nexport const PI: f32 = 3.14;\nexport { shade };\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            refs("export { a, b } from \"./a.wgsl\";\nexport * from './b.wgsl';\n"),
            ["re_export ./a.wgsl", "re_export ./b.wgsl"]
        );
        // What follows an export list is read on.
        assert_eq!(
            refs("export { a }\nimport { b } from \"./b.wgsl\";\n"),
            ["import ./b.wgsl"]
        );
    }

    #[test]
    fn comments_hide_import_lines() {
        assert_eq!(
            refs(
                "// import { a } from \"./a.wgsl\";\n/* import { b } from \"./b.wgsl\";\n /* \
                 nested */ import { c } from \"./c.wgsl\"; */\nimport /* names */ { d } /* from \
                 */ from /* here */ \"./d.wgsl\";\n"
            ),
            ["import ./d.wgsl"]
        );
        // An unterminated comment ends the file.
        assert_eq!(
            refs("/* import { a } from \"./a.wgsl\";\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn code_is_not_an_import() {
        // The word inside other code, as part of a name, after a member access.
        assert_eq!(
            refs(
                "fn important() -> f32 { return 1.0; }\nlet x = a.import;\nvar exported: \
                 f32;\n@fragment fn main() -> @location(0) vec4f { return vec4f(1.0); }\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn unreadable_import_statements_are_recorded() {
        // Another module syntax.
        assert_eq!(
            refs("import package::util::hash;\nfn f() {}\n"),
            ["! import package"]
        );
        assert_eq!(
            refs("import super::scene::{ Scene, project };\n"),
            ["! import super"]
        );
        // No `from`, a string that does not end on its line, an empty specifier, the
        // end of the file.
        assert_eq!(refs("import { a } \"./a.wgsl\";"), ["! import { a } \""]);
        assert_eq!(
            refs("import { a } from \"./a.wgsl\nfn f() {}"),
            ["! import { a } from \"./a.wgsl"]
        );
        assert_eq!(
            refs("import { a } from \"\";"),
            ["! import { a } from \"\""]
        );
        assert_eq!(refs("import { a } from"), ["! import { a } from"]);
        assert_eq!(refs("import { a };"), ["! import { a }"]);
        // The readable lines of the same file are still references.
        assert_eq!(
            refs("import a::b;\nimport { c } from \"./c.wgsl\";\n"),
            ["import ./c.wgsl", "! import a"]
        );
        // Preprocessor lines that include a file; other directives are skipped whole.
        assert_eq!(
            refs(
                "#import \"./common.wgsl\"\n  #include <noise>\n#define import 1\n#if X\nfn f() \
                 {}\n"
            ),
            ["! #import \"./common.wgsl\"", "! #include <noise>"]
        );
        // A statement without a module ends where the next one starts.
        assert_eq!(
            refs("import { a }\nimport { b } from \"./b.wgsl\"\nimport { c } from \"./c.wgsl\""),
            ["import ./b.wgsl", "import ./c.wgsl", "! import { a }\n"]
        );
        // A re-export with a string it cannot read; an export list at the end of the
        // file is none.
        assert_eq!(
            refs("export { a } from \"./a.wgsl\nfn f() {}"),
            ["! export { a } from \"./a.wgsl"]
        );
        assert_eq!(refs("export { a }"), Vec::<String>::new());
    }

    #[test]
    fn arbitrary_bytes_do_not_panic() {
        for src in [
            &b"import"[..],
            b"import {",
            b"import \xff\xfe from \"\xff\"",
            b"export * from '",
            b"/*/ import",
            b"import { a } from \"x\" import",
            b"\"import\" 'export' from",
        ] {
            let mut sink = RefSink::new();
            scan(src, &mut sink);
        }
    }
}
