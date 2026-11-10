//! Recognizer unit tests.

use turborepo_trace_core::UncertaintyKind;

use super::*;

/// Renders refs as `kind spec [flags]`.
fn refs_in(src: &str, dialect: Dialect) -> Vec<String> {
    let ex = scan(src.as_bytes(), dialect, ScanOptions::default());
    ex.refs
        .iter()
        .map(|r| {
            let mut s = format!("{} {}", r.kind, r.spec);
            for f in r.flags.names() {
                s.push(' ');
                s.push_str(f);
            }
            s
        })
        .collect()
}

fn js(src: &str) -> Vec<String> {
    refs_in(src, Dialect::JS)
}

fn ts(src: &str) -> Vec<String> {
    refs_in(src, Dialect::TS)
}

fn none() -> Vec<String> {
    Vec::new()
}

#[test]
fn static_imports() {
    assert_eq!(js("import x from 'a'"), ["import a"]);
    assert_eq!(js("import {a as b, c} from \"a\""), ["import a"]);
    assert_eq!(js("import * as ns from 'a'"), ["import a"]);
    assert_eq!(js("import 'a'"), ["import a"]);
    assert_eq!(js("import x, {y} from 'a'"), ["import a"]);
    assert_eq!(js("import x, * as y from 'a'"), ["import a"]);
    assert_eq!(js("import {} from 'a'"), ["import a"]);
    assert_eq!(
        js("import {default as x, 'str-name' as y} from 'a'"),
        ["import a"]
    );
    assert_eq!(js("import from from 'a'"), ["import a"]);
    assert_eq!(js("import `a`"), ["import a"]);
    assert_eq!(
        js("import a from 'a'\nimport b from 'b';import c from 'c'"),
        ["import a", "import b", "import c"]
    );
    // Escapes are decoded.
    assert_eq!(js(r"import '\x61\u{62}'"), ["import ab"]);
}

#[test]
fn import_attributes() {
    assert_eq!(
        js("import data from './d.json' with { type: 'json' }; import 'b'"),
        ["import ./d.json", "import b"]
    );
    assert_eq!(
        js("import data from './d.json' assert { type: 'json' }"),
        ["import ./d.json"]
    );
    assert_eq!(
        js("export * from './d.json' with { type: 'json' }"),
        ["re_export ./d.json"]
    );
    assert_eq!(
        js("import('./d.json', { with: { type: 'json' } })"),
        ["dynamic_import ./d.json lazy"]
    );
}

#[test]
fn type_only_imports() {
    assert_eq!(ts("import type T from 'a'"), ["import a type_only"]);
    assert_eq!(ts("import type {A, B} from 'a'"), ["import a type_only"]);
    assert_eq!(ts("import type * as ns from 'a'"), ["import a type_only"]);
    // `type` as a default import name.
    assert_eq!(ts("import type from 'a'"), ["import a"]);
    assert_eq!(ts("import type, {a} from 'a'"), ["import a"]);
    assert_eq!(ts("import type from from 'a'"), ["import a type_only"]);
    // Inline `type` qualifiers never make the whole import type-only (a
    // deliberate choice: see the module docs).
    assert_eq!(ts("import {type A} from 'a'"), ["import a"]);
    assert_eq!(ts("import {type A, b} from 'a'"), ["import a"]);
    assert_eq!(ts("import {type as} from 'a'"), ["import a"]);
    assert_eq!(ts("import {type as as as} from 'a'"), ["import a"]);
    // Flow.
    assert_eq!(js("import typeof T from 'a'"), ["import a type_only"]);
    // Phase modifiers.
    assert_eq!(js("import defer * as ns from 'a'"), ["import a"]);
    assert_eq!(js("import source x from 'a'"), ["import a"]);
    assert_eq!(js("import source from 'a'"), ["import a"]);
}

#[test]
fn re_exports() {
    assert_eq!(js("export * from 'a'"), ["re_export a"]);
    assert_eq!(js("export * as ns from 'a'"), ["re_export a"]);
    assert_eq!(js("export * as 'string name' from 'a'"), ["re_export a"]);
    assert_eq!(js("export {a, b as c, default} from 'a'"), ["re_export a"]);
    assert_eq!(js("export {default as x} from 'a'"), ["re_export a"]);
    assert_eq!(ts("export type {A} from 'a'"), ["re_export a type_only"]);
    assert_eq!(ts("export type * from 'a'"), ["re_export a type_only"]);
    assert_eq!(ts("export type * as T from 'a'"), ["re_export a type_only"]);
    assert_eq!(ts("export {type A} from 'a'"), ["re_export a"]);
    // Not re-exports.
    assert_eq!(
        js("export {a, b}; export default 'a'; export const x = 'a'"),
        none()
    );
    assert_eq!(ts("export type A = 'a'"), none());
}

#[test]
fn ts_import_equals() {
    assert_eq!(ts("import fs = require('fs')"), ["require fs"]);
    assert_eq!(ts("export import fs = require('fs')"), ["require fs"]);
    assert_eq!(ts("import type T = require('t')"), ["require t type_only"]);
    assert_eq!(ts("import A = B.C; import D = require"), none());
}

#[test]
fn triple_slash_directives() {
    assert_eq!(
        ts(
            "/// <reference path=\"./a.d.ts\" />\n/// <reference types='node' />\n/// <reference \
             lib=\"es2020\" />\n/// <reference path=\"globals.d.ts\"/>\nlet x;\n/// <reference \
             path=\"late.d.ts\" />"
        ),
        [
            "import ./a.d.ts type_only",
            "import node type_only",
            "import ./globals.d.ts type_only"
        ]
    );
    assert_eq!(js("/// <reference no-default-lib=\"true\"/>"), none());
    assert_eq!(js("/// just a comment with reference"), none());
    assert_eq!(js("/// <reference path=\"\" />"), none());
    assert_eq!(js("/// <reference path=\"unterminated />"), none());
    // Span covers the quoted value.
    let src = "/// <reference path=\"./a\" />";
    let ex = scan(src.as_bytes(), Dialect::TS, ScanOptions::default());
    assert_eq!(&src[ex.refs[0].span.range()], "\"./a\"");
}

#[test]
fn dynamic_imports() {
    assert_eq!(js("import('a')"), ["dynamic_import a lazy"]);
    assert_eq!(js("await import(`a`)"), ["dynamic_import a lazy"]);
    assert_eq!(js("import(('a'))"), ["dynamic_import a lazy"]);
    assert_eq!(
        js("const m = () => import('./m').then(x => x.default)"),
        ["dynamic_import ./m lazy"]
    );
    assert_eq!(
        js("try { await import('a') } catch {}"),
        ["dynamic_import a optional lazy"]
    );
    assert_eq!(
        ts("type T = typeof import('a')"),
        ["dynamic_import a lazy type_only"]
    );
    // Not dynamic imports.
    assert_eq!(
        js("import.meta.url; x.import('a'); a?.import('b'); import()"),
        none()
    );
    assert_eq!(js("class A { import(x) { return x } }"), none());
    assert_eq!(js("({ import: 'a' })"), none());
}

#[test]
fn requires() {
    assert_eq!(js("require('a')"), ["require a"]);
    assert_eq!(js("const a = require(\"a\")"), ["require a"]);
    assert_eq!(js("require.resolve('a')"), ["require a"]);
    assert_eq!(js("module.require('a')"), ["require a"]);
    assert_eq!(js("require(`a`)"), ["require a"]);
    assert_eq!(js("require('a', 'ignored')"), ["require a"]);
    assert_eq!(
        js("f(require('a'), require('b'))"),
        ["require a", "require b"]
    );
    assert_eq!(
        js("require('a').b.c(require('c'))"),
        ["require a", "require c"]
    );
    assert_eq!(
        js("var a = require('a'), b = require('b')"),
        ["require a", "require b"]
    );
    // Not requires.
    for src in [
        "obj.require('a')",
        "obj?.require('a')",
        "x.module.require('a')",
        "function require(a) { return a }",
        "const require = createRequire(import.meta.url)",
        "({ require: 'a' })",
        "({ require(a) { return a } })",
        "typeof require === 'function'",
        "require.cache; require.main",
        "require.resolve.paths('a')",
        "require()",
        "require(['a', 'b'], function (a, b) {})",
        "interface M { require(id: string): any; require(id?: string): any }",
        "'require(\"a\")'; // require('b')\n/* require('c') */ `require('d')`",
        "x = /require('a')/",
        "requirex('a'); xrequire('b'); __webpack_require__('c')",
    ] {
        assert_eq!(js(src), none(), "{src}");
    }
}

#[test]
fn optional_requires_in_try() {
    assert_eq!(
        js("try { require('a') } catch (e) { require('b') } finally { require('c') }"),
        ["require a optional", "require b", "require c"]
    );
    assert_eq!(
        js("try { try {} catch { require('a') } } catch {}"),
        ["require a optional"]
    );
    assert_eq!(
        js("try { var f = function () { return require('a') } } catch {}"),
        ["require a optional"]
    );
    assert_eq!(js("try {} catch {} require('a')"), ["require a"]);
    // Only a handler that swallows the error makes the load optional.
    assert_eq!(js("try { require('a') } finally {}"), ["require a"]);
    assert_eq!(
        js("try { require('a') } catch (e) { throw new Error('install a') }"),
        ["require a"]
    );
    assert_eq!(
        js("try { try { require('a') } catch {} } catch (e) { throw e }"),
        ["require a optional"]
    );
    assert_eq!(
        js("try { require.resolve('a'); import('b') } catch { /* not installed */ }"),
        ["require a optional", "dynamic_import b optional lazy"]
    );
    assert_eq!(js("x.try { require('a') }"), ["require a"]);
}

#[test]
fn computed_specifiers() {
    let ex = scan(
        b"require(name); import('./locale/' + lang); require(`./x/${y}`); require(...args)",
        Dialect::JS,
        ScanOptions::default(),
    );
    let got: Vec<_> = ex
        .refs
        .iter()
        .map(|r| {
            let u = r.uncertainty.as_ref().unwrap();
            (r.kind, r.spec.to_string(), u.kind, u.detail.to_string())
        })
        .collect();
    use UncertaintyKind::{ComputedSpecifier as C, PartialSpecifier as P};
    let s = |x: &str| x.to_string();
    assert_eq!(
        got,
        [
            (RefKind::Require, s("<unknown>"), C, s("name")),
            (RefKind::DynamicImport, s("./locale/*"), P, s("lang")),
            (RefKind::Require, s("./x/*"), P, s("y")),
            (RefKind::Require, s("<unknown>"), C, s("...args")),
        ]
    );
    // The uncertainty span is the argument.
    let src = "x = require( a.b + c )";
    let ex = scan(src.as_bytes(), Dialect::JS, ScanOptions::default());
    let u = ex.refs[0].uncertainty.as_ref().unwrap();
    assert_eq!(&src[u.span.range()], "a.b + c");
    assert_eq!(&src[ex.refs[0].span.range()], "require( a.b + c )");
}

#[test]
fn computed_argument_tokens_are_still_scanned() {
    assert_eq!(
        js("require(cond ? require('a') : require('b'))"),
        ["require <unknown>", "require a", "require b"]
    );
    // The load is the file `require.resolve` finds.
    assert_eq!(
        js("import(`${require.resolve('a')}`)"),
        ["dynamic_import require.resolve('a') lazy", "require a"]
    );
    assert_eq!(
        js("import(`${require.resolve('a')}x`)"),
        ["dynamic_import <unknown> lazy", "require a"]
    );
}

#[test]
fn long_computed_argument_is_truncated() {
    let arg = (0..2000)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let src = format!("require({arg}); require('after')");
    let ex = scan(src.as_bytes(), Dialect::JS, ScanOptions::default());
    assert_eq!(ex.refs.len(), 2);
    let u = ex.refs[0].uncertainty.as_ref().unwrap();
    assert!(u.detail.ends_with('…'));
    assert!(u.detail.len() < 300);
    assert_eq!(ex.refs[1].spec, Specifier::literal("after"));
}

#[test]
fn spans() {
    let src = "import x from './a';\nexport * from \
               './b'\nrequire('./c')\nimport('./d')\nrequire.resolve('./e')\nmodule.require('./f')";
    let ex = scan(src.as_bytes(), Dialect::JS, ScanOptions::default());
    let texts: Vec<_> = ex.refs.iter().map(|r| &src[r.span.range()]).collect();
    assert_eq!(
        texts,
        [
            "import x from './a'",
            "export * from './b'",
            "require('./c')",
            "import('./d')",
            "require.resolve('./e')",
            "module.require('./f')",
        ]
    );
}

#[test]
fn nothing_inside_strings_comments_regex_templates_jsx() {
    let src = r#"
        // import a from 'a'
        /* require('b') */
        const s = "import c from 'c'";
        const t = `export * from 'd' ${x} require('e')`;
        const r = /import f from 'f'/;
        const j = <div>import g from 'g'; require('h') {/* require('i') */}</div>;
        const k = <a title="require('j')" />;
    "#;
    assert_eq!(refs_in(src, Dialect::JSX), none());
    assert_eq!(js(src), none());
}

#[test]
fn jsx_containers_are_scanned() {
    let src = "x = <div>{require('a')} it's <b c={import('b')} /></div>; require('c')";
    assert_eq!(
        refs_in(src, Dialect::JSX),
        ["require a", "dynamic_import b lazy", "require c"]
    );
    assert_eq!(js(src), ["require a", "dynamic_import b lazy", "require c"]);
}

#[test]
fn js_jsx_heuristic_fallback() {
    // Flow generic arrow: JSX reading fails, the re-run without JSX finds both
    // refs.
    let src = "const f = <T>(x: T): T => x;\nimport a from 'a'\nconst b = require('b')";
    assert_eq!(js(src), ["import a", "require b"]);
    let ex = scan(src.as_bytes(), Dialect::JS, ScanOptions::default());
    assert!(ex.diagnostics.is_empty());
}

#[test]
fn dts_refs_are_type_only() {
    assert_eq!(
        refs_in(
            "import x from 'a'; export * from 'b'; declare const c: typeof import('c')",
            Dialect::DTS
        ),
        [
            "import a type_only",
            "re_export b type_only",
            "dynamic_import c lazy type_only"
        ]
    );
}

#[test]
fn ts_specific_syntax() {
    let src = r#"
        import type { A } from './types';
        import { b, type C } from './b';
        export type { D } from './d';
        const x = y! / 2 / 3; require('e');
        function f<T extends object>(a: T): Promise<T> { return import('./g') }
        let z = <any>require('h');
        declare module 'not-a-ref' { export const v: number }
        enum E { A = 1 }
        abstract class K<T> implements I { private m?: string; }
        const n = require('n') as typeof import('n2');
    "#;
    assert_eq!(
        ts(src),
        [
            "import ./types type_only",
            "import ./b",
            "re_export ./d type_only",
            "require e",
            "dynamic_import ./g lazy",
            "require h",
            "require n",
            "dynamic_import n2 lazy type_only",
        ]
    );
}

#[test]
fn tsx_generics_and_jsx() {
    let src = "const f = <T,>(x: T) => require('a');\nconst g = () => <div>it's</div>;\nimport 'b'";
    assert_eq!(refs_in(src, Dialect::TSX), ["require a", "import b"]);
}

#[test]
fn hashbang_and_bom() {
    assert_eq!(js("#!/usr/bin/env node\nrequire('a')"), ["require a"]);
    assert_eq!(js("\u{feff}import 'a'"), ["import a"]);
}

#[test]
fn prefilter_skip() {
    let ex = scan(b"const x = 1;", Dialect::JS, ScanOptions::default());
    assert!(ex.skipped);
    let ex = scan(b"import 'a'", Dialect::JS, ScanOptions::default());
    assert!(!ex.skipped);
}

#[test]
fn malformed_input_reports_once_and_continues() {
    let src = "x = 'unterminated\ny = \"also\nrequire('a'); }}} require('b'); /* never closed";
    let ex = scan(src.as_bytes(), Dialect::JS, ScanOptions::default());
    assert_eq!(ex.refs.len(), 2);
    assert_eq!(ex.diagnostics.len(), 1);
    assert_eq!(ex.diagnostics[0].kind, UncertaintyKind::ParseIncomplete);
}

#[test]
fn undecodable_specifier() {
    let src = b"require('\xff\xfe')";
    let ex = scan(src, Dialect::JS, ScanOptions::default());
    assert_eq!(ex.refs.len(), 1);
    assert_eq!(ex.refs[0].spec, Specifier::Unknown);
    assert_eq!(
        ex.refs[0].uncertainty.as_ref().unwrap().kind,
        UncertaintyKind::ParseIncomplete
    );
}

fn bindings(src: &str, dialect: Dialect) -> Vec<String> {
    let ex = scan(
        src.as_bytes(),
        dialect,
        ScanOptions {
            bindings: true,
            ..Default::default()
        },
    );
    ex.bindings
        .iter()
        .map(|b| {
            let imported = match &b.imported {
                Imported::Namespace => "*".to_string(),
                Imported::Default => "default".to_string(),
                Imported::Named(n) => n.to_string(),
            };
            format!(
                "{} = {}:{} ({:?}{})",
                b.local,
                b.module,
                imported,
                b.source,
                if b.type_only { ", type" } else { "" }
            )
        })
        .collect()
}

#[test]
fn top_level_bindings() {
    let src = r#"
        import fs, { readFile as rf, promises } from 'fs';
        import * as path from 'node:path';
        import { type Stats } from 'fs';
        import type { Dirent } from 'fs';
        import { 'quoted' as q } from 'q';
        const cp = require('child_process');
        const { spawn, exec: run } = require('child_process'), os = require('os');
        var join = require('path').join;
        let fse = await import('fs-extra');
        function f() { const inner = require('inner'); }
        const notReq = foo('x');
        const { a: { nested } } = require('nested');
    "#;
    assert_eq!(
        bindings(src, Dialect::TS),
        [
            "fs = fs:default (Import)",
            "rf = fs:readFile (Import)",
            "promises = fs:promises (Import)",
            "path = node:path:* (Import)",
            "Stats = fs:Stats (Import, type)",
            "Dirent = fs:Dirent (Import, type)",
            "q = q:quoted (Import)",
            "cp = child_process:* (Require)",
            "spawn = child_process:spawn (Require)",
            "run = child_process:exec (Require)",
            "os = os:* (Require)",
            "join = path:join (Require)",
            "fse = fs-extra:* (DynamicImport)",
        ]
    );
    assert_eq!(
        bindings("import fs = require('fs')", Dialect::TS),
        ["fs = fs:* (Require)"]
    );
    // Off by default.
    let ex = scan(b"import fs from 'fs'", Dialect::JS, ScanOptions::default());
    assert!(ex.bindings.is_empty());
}

#[test]
fn call_followed_by_block_is_not_a_method() {
    assert_eq!(js("require('./a')\n{}"), ["require ./a"]);
    assert_eq!(js("require('./a') {}"), ["require ./a"]);
    assert_eq!(js("require(x)\n{}"), ["require <unknown>"]);
    assert_eq!(js("({ require(x) {} })"), none());
}

#[test]
fn typescript_methods_named_require_with_a_return_type() {
    // A method with a destructured default parameter and a return type is not a
    // call.
    assert_eq!(
        ts(
            "const o = {\n  require({ enableFallback = true } = {}): Exports {\n    return \
             load();\n  },\n};"
        ),
        none()
    );
    assert_eq!(
        ts("const o = { a: 1, async require(opts: Opts): Promise<T> { return x; } };"),
        none()
    );
    assert_eq!(
        ts(
            "class C {\n  private static require(x): T { return x; }\n  require(y): T { return y; \
             }\n}"
        ),
        none()
    );
    // Calls followed by a colon stay calls: a ternary branch, a `case` label.
    assert_eq!(ts("const m = c ? require(x) : y;"), ["require <unknown>"]);
    assert_eq!(
        ts("const m = c ? require('./a') : { b: 1 };"),
        ["require ./a"]
    );
    assert_eq!(js("({ k: c ? require(x) : y })"), ["require <unknown>"]);
}

// ---- evaluator, APIs, loaders, type positions ----

/// Renders refs as `kind spec [@cwd] [flags] [{uncertainty}]`.
fn full(src: &str, dialect: Dialect) -> Vec<String> {
    let ex = scan_named(
        src.as_bytes(),
        dialect,
        ScanOptions::default(),
        Some("main.js"),
    );
    ex.refs
        .iter()
        .map(|r| {
            let mut s = format!("{} {}", r.kind, r.spec);
            match r.base {
                Base::FileDir => {}
                Base::Cwd => s.push_str(" @cwd"),
                ref b => s.push_str(&format!(" @{b:?}")),
            }
            for f in r.flags.names() {
                s.push(' ');
                s.push_str(f);
            }
            if let Some(u) = &r.uncertainty {
                s.push_str(&format!(" {{{}: {}}}", u.kind, u.detail));
            }
            s
        })
        .collect()
}

fn fj(src: &str) -> Vec<String> {
    full(src, Dialect::JS)
}

fn ft(src: &str) -> Vec<String> {
    full(src, Dialect::TS)
}

#[test]
fn evaluated_module_specifiers() {
    assert_eq!(fj("require('u' + 'rl')"), ["require url"]);
    assert_eq!(
        fj("require(String.raw`./snippets/x/tracer_logger.js`)"),
        ["require ./snippets/x/tracer_logger.js"]
    );
    assert_eq!(
        fj(
            "require(inAWSLambda && !usingLambdaExtension ? './exporters/log' : \
             './exporters/agent')"
        ),
        ["require ./exporters/log | ./exporters/agent conditional"]
    );
    assert_eq!(
        fj("function load(path) { return require(path) }"),
        ["require <unknown> {computed_specifier: path; parameter load#0}"]
    );
    assert_eq!(
        fj("require('./plugins/' + name + '.js')"),
        ["require ./plugins/*.js {partial_specifier: name}"]
    );
    assert_eq!(
        fj("require(name + '.js')"),
        ["require <unknown> {computed_specifier: name + '.js'}"]
    );
    assert_eq!(
        fj("import(`./locales/${lang}/messages.json`)"),
        ["dynamic_import ./locales/*/messages.json lazy {partial_specifier: lang}"]
    );
    assert_eq!(
        fj("require(process.env.PLUGIN || './default')"),
        [
            "require ./default conditional",
            "require <unknown> conditional {env_dependent: process.env.PLUGIN || './default'; env \
             PLUGIN}"
        ]
    );
    assert_eq!(
        fj("require(__dirname + '/lib/x.js')"),
        ["require ./lib/x.js"]
    );
    assert_eq!(
        fj("const path = require('path'); require(path.join(__dirname, '..', 'y'))"),
        ["require path", "require ../y"]
    );
    assert_eq!(
        fj("require.resolve('a' + '/package.json')"),
        ["require a/package.json"]
    );
}

#[test]
fn top_level_constants() {
    let src = "
const path = require('node:path');
const DIR = path.join(__dirname, 'locales');
const EXT = '.json';
const NAME = `plugin`;
let notConst = './nope';
require('./' + NAME);
require(notConst);
module.exports = (l) => require(path.join(DIR, l + EXT));
";
    assert_eq!(
        fj(src),
        [
            "require node:path",
            "asset locales",
            "require ./plugin",
            "require ./nope",
            "require ./locales/*.json {partial_specifier: l}",
        ]
    );
    // TS annotations and multiple declarators.
    assert_eq!(
        ft("const A: string = 'a', B = A + '/b';\nrequire(B)"),
        ["require a/b"]
    );
    // Function-local constants are the value-flow pass's.
    assert_eq!(
        fj("function f() { const X = './x'; return require(X) }"),
        ["require ./x"]
    );
}

#[test]
fn fs_apis_through_bindings() {
    let src = "
import fs from 'fs';
import * as nfs from 'node:fs';
import { readFileSync as rfs, promises as fsp } from 'fs';
import { readdir } from 'fs/promises';
import path from 'path';
fs.readFileSync(path.join(__dirname, '../data', name + '.json'));
nfs.existsSync(__dirname + '/x.txt');
rfs('./config.json', 'utf8');
fsp.readFile(new URL('./a.txt', import.meta.url));
readdir(path.resolve(__dirname, 'templates'));
fs.promises.stat('/etc/hosts');
other.readFileSync('./nope');
fs.writeFileSync('./nope');
";
    assert_eq!(
        fj(src),
        [
            "import fs",
            "import node:fs",
            "import fs",
            "import fs/promises",
            "import path",
            "fs_read ../data/*.json {partial_specifier: name}",
            "fs_read x.txt optional names",
            "fs_read ./config.json @cwd {cwd_dependent: './config.json'}",
            "fs_read a.txt",
            "fs_read_dir templates names",
            // A `stat` whose result is dropped only tells whether the path is there.
            "fs_read /etc/hosts optional names",
        ]
    );
}

#[test]
fn fs_apis_cjs_forms() {
    let src = "
const { readFileSync: r, statSync } = require('fs');
const fsp = require('fs').promises;
const gfs = require('graceful-fs');
r(__dirname + '/a');
statSync(process.cwd() + '/b');
fsp.readdir(dir);
gfs.createReadStream(require.resolve('./c.bin'));
require('fs').readFileSync(__filename);
require('fs-extra').readJsonSync('/abs.json');
";
    assert_eq!(
        full(src, Dialect::JS),
        [
            "require fs",
            "require fs",
            "require graceful-fs",
            "fs_read a",
            "fs_read b @cwd optional names {cwd_dependent: process.cwd() + '/b'}",
            "fs_read_dir <unknown> names {computed_specifier: dir}",
            "fs_read c.bin",
            "require ./c.bin",
            "require fs",
            "fs_read main.js",
            "require fs-extra",
            "fs_read /abs.json",
        ]
    );
}

#[test]
fn function_local_bindings_end_with_their_scope() {
    let src = "
function a() { var f = require('fs'); f.readFileSync('/x'); }
function b(f) { f.readFileSync('/y'); }
!function () { const { readFileSync } = require('fs'); readFileSync('/z') }();
readFileSync('/w');
";
    assert_eq!(
        fj(src),
        ["require fs", "fs_read /x", "require fs", "fs_read /z",]
    );
}

#[test]
fn workers_and_child_processes() {
    let src = "
import { Worker } from 'node:worker_threads';
import * as cp from 'child_process';
import { fork } from 'child_process';
new Worker(new URL('./worker.js', import.meta.url));
new Worker(path.join(__dirname, 'w2.js'));
cp.fork('./child.js');
fork(require.resolve('./child2.js'), ['--x']);
";
    assert_eq!(
        fj(src),
        [
            "import node:worker_threads",
            "import child_process",
            "import child_process",
            "worker worker.js",
            "worker w2.js {partial_specifier: `path.join(__dirname, 'w2.js')` assumed to be \
             `path.join`}",
            "child_process ./child.js @cwd {cwd_dependent: './child.js'}",
            "child_process child2.js",
            "require ./child2.js",
        ]
    );
    // A global Worker (no import), and `new URL` assets anywhere.
    assert_eq!(
        fj(
            "const w = new Worker('./w.js'); const wasm = new URL('./a.wasm', import.meta.url); fetch(new URL(`./img/${n}.png`, import.meta.url)); new URL('https://x.dev', import.meta.url); new URL('./rel', base)"
        ),
        [
            "worker ./w.js @cwd {cwd_dependent: './w.js'}",
            "fs_read a.wasm",
            "fs_read img/*.png {partial_specifier: n}",
        ]
    );
}

#[test]
fn loaders_aliases_and_shadowing() {
    // Aliased require: still evaluated, marked.
    assert_eq!(
        fj("const r = require; r('./a'); r('./b' + x)"),
        [
            "require ./a {aliased_loader: `r` is an alias of `require`}",
            "require ./b* {aliased_loader: `r` is an alias of `require`}",
        ]
    );
    // createRequire: plain require semantics.
    assert_eq!(
        fj("import { createRequire } from 'node:module';\nconst req = \
            createRequire(import.meta.url);\nreq('./c');\nreq.resolve('./d')"),
        ["import node:module", "require ./c", "require ./d"]
    );
    assert_eq!(
        fj(
            "import module from 'module'; const require = module.createRequire(import.meta.url); \
             require('./e')"
        ),
        ["import module", "require ./e"]
    );
    // A bundle wrapper shadows `require` for its body only.
    let src = "
(function (modules) {})({
  1: [function (require, module, exports) { require('./inner'); }, {}],
});
require('./outer');
function own(require) { return require('./param') }
";
    assert_eq!(
        fj(src),
        [
            "require ./inner {aliased_loader: `require` is shadowed by a local declaration \
             (bundle wrapper?)}",
            "require ./outer",
            "require ./param {aliased_loader: `require` is shadowed by a local declaration \
             (bundle wrapper?)}",
        ]
    );
    // Passing `require` as an argument or testing it is not shadowing.
    assert_eq!(
        fj("factory(require, exports); if (require) { require('./x') }"),
        ["require ./x"]
    );
    // `var require = ...` shadows the rest of the scope.
    assert_eq!(
        fj("function f() { var require = g(); require('./y') } require('./z')"),
        [
            "require ./y {aliased_loader: `require` is shadowed by a local declaration (bundle \
             wrapper?)}",
            "require ./z",
        ]
    );
}

/// `createRequire(a)` gives a `require` that resolves as a module at `a` would: the
/// file's own for `import.meta.url` / `__filename` (and any file in its directory), another
/// place for a known path, and unknown otherwise. Never a reference from the wrong place.
// rustfmt's `format_strings` would change this item's string literals.
#[rustfmt::skip]
#[test]
fn create_require_searches_from_its_argument() {
    const M: &str =
        "const { createRequire } = require('node:module');\nconst path = require('node:path');\n";
    let refs = |body: &str| -> Vec<String> {
        fj(&format!("{M}{body}"))
            .into_iter()
            .skip(2)
            .map(|r| r.replace("@Language(\"require.resolve\")", "@resolved"))
            .collect()
    };
    // A sibling directory: relative requests are joined onto it, packages are searched
    // from it. The argument itself is no asset.
    assert_eq!(
        refs(
            "const other = createRequire(path.join(__dirname, '../other/package.json'));\n\
             other('./x.js'); other('dep'); other.resolve('dep/package.json');"
        ),
        [
            "require ../other/x.js",
            "require require.resolve('dep', from '../other') @resolved",
            "require require.resolve('dep/package.json', from '../other') @resolved",
        ]
    );
    // A file in the file's own directory: the file's own `require`.
    assert_eq!(
        refs(
            "const a = createRequire(path.join(__dirname, 'package.json'));\n\
             const b = createRequire(__dirname + '/');\n\
             const c = createRequire(__filename);\n\
             a('./x'); b('dep'); c.resolve('./y');"
        ),
        ["require ./x", "require dep", "require ./y"]
    );
    // `createRequire(dir)` without a trailing `/` searches from the parent directory.
    assert_eq!(
        refs(
            "createRequire(__dirname)('./x'); createRequire(path.join(__dirname, 'sub/'))('./y');"
        ),
        ["require ../x", "require ./sub/y"]
    );
    // Inline calls.
    assert_eq!(
        refs(
            "createRequire(path.join(__dirname, '..', 'package.json'))('./src/b.cjs');\n\
             createRequire(__filename).resolve('dep');\n\
             createRequire(require.resolve('eve/package.json'))('dep');\n\
             createRequire(require.resolve('eve/package.json')).resolve('./lib/x.js');"
        ),
        [
            "require ../src/b.cjs",
            "require dep",
            "require require.resolve('dep', from require.resolve('eve/package.json')/'..') @resolved",
            "require eve/package.json",
            "require require.resolve('eve/package.json')/../lib/x.js @resolved",
            "require eve/package.json",
        ]
    );
    // Under the usual name.
    assert_eq!(
        refs(
            "function f() {\n  const require = createRequire(path.join(__dirname, '../other/noop.js'));\n  \
             return [require('./x.js'), require.resolve('dep')];\n}\nrequire('./own');"
        ),
        [
            "require ../other/x.js",
            "require require.resolve('dep', from '../other') @resolved",
            "require ./own",
        ]
    );
    // An argument that cannot be evaluated: unknown, with a record that shows the call.
    for (body, call) in [
        (
            "module.exports = (d) => createRequire(d)('dep');",
            "createRequire(d)('dep')",
        ),
        (
            "module.exports = (d) => { const r = createRequire(d); return r('./x'); };",
            "r('./x')",
        ),
        (
            "module.exports = (d) => { const require = createRequire(d); return require('./x'); };",
            "require('./x')",
        ),
        (
            "module.exports = (d) => createRequire(d).resolve('dep');",
            "createRequire(d).resolve('dep')",
        ),
        (
            "const r = createRequire(process.cwd() + '/'); r('dep');",
            "r('dep')",
        ),
    ] {
        let out = refs(body);
        assert!(
            out.iter().all(|r| r.starts_with("require <unknown>")),
            "{body}: {out:?}"
        );
        assert!(
            out.iter()
                .any(|r| r.contains(&format!("computed_specifier: {call}"))),
            "{body}: {out:?}"
        );
    }
    // A builtin module under the usual name is the builtin, and its binding is tracked.
    assert_eq!(
        refs(
            "function f(d) {\n  const require = createRequire(d);\n  const fs = require('fs');\n  \
             return fs.readFileSync(path.join(__dirname, 'a.txt'));\n}"
        ),
        ["require fs", "fs_read a.txt"]
    );
    // An absolute path is the same file for every `require` function.
    assert_eq!(
        refs(
            "module.exports = (d) => {\n  const r = createRequire(d);\n  \
             return [r(path.join(__dirname, 'a.js')), r.resolve(path.join(__dirname, 'b.js'))];\n};"
        ),
        ["require ./a.js", "require ./b.js"]
    );
    // `node:` names a builtin module for every `require` function.
    assert_eq!(
        refs(
            "module.exports = (d) => [createRequire(d)('node:fs'), createRequire(d)(d ? 'node:os' : 'x')];"
        ),
        [
            "require node:fs",
            "require node:os conditional",
            "require <unknown> conditional {computed_specifier: createRequire(d)(d ? 'node:os' : 'x'); parameter module.exports#0}",
        ]
    );
    // A name bound twice, once to something unknown: the known place, and a record.
    assert_eq!(
        refs("let r = createRequire(__filename); r = createRequire(elsewhere()); r('dep');"),
        [
            "require dep conditional",
            "require <unknown> conditional {computed_specifier: r('dep')}",
        ]
    );
    // A name bound twice: both places.
    assert_eq!(
        refs(
            "let r = createRequire(__filename);\n\
             if (process.env.X) r = createRequire(path.join(__dirname, '../other/a.js'));\n\
             r('./x.js');"
        ),
        ["require ./x.js | ../other/x.js conditional"]
    );
    // An inner declaration of the name is its own `require`.
    assert_eq!(
        refs(
            "const r = createRequire(path.join(__dirname, '../other/a.js'));\n\
             function f() { const r = createRequire(__filename); return r('./in'); }\n\
             r('./out');"
        ),
        ["require ./in", "require ../other/out"]
    );
}

/// `require.resolve(request, { paths })` searches from each entry instead of
/// the file.
#[test]
fn require_resolve_with_options() {
    const M: &str = "const path = require('node:path');\n";
    let refs = |body: &str| -> Vec<String> {
        fj(&format!("{M}{body}"))
            .into_iter()
            .skip(1)
            .map(|r| r.replace("@Language(\"require.resolve\")", "@resolved"))
            .collect()
    };
    assert_eq!(
        refs("require(require.resolve('dep', { paths: [require.resolve('eve/package.json')] }));"),
        [
            "require require.resolve('dep', from require.resolve('eve/package.json')) @resolved",
            "require require.resolve('dep', from require.resolve('eve/package.json')) @resolved",
            "require eve/package.json",
        ]
    );
    // One alternative per entry; an entry in the file's own directory is the file.
    assert_eq!(
        refs("require.resolve('dep', { paths: [__dirname, path.join(__dirname, '../other')] });"),
        [
            "require require.resolve('dep') | require.resolve('dep', from '../other') @resolved \
             conditional",
        ]
    );
    // A relative request is joined onto the entry.
    assert_eq!(
        refs(
            "require.resolve('./x.js', { paths: [path.join(__dirname, '../other')] \
             });\nrequire.resolve('./lib/y', { paths: [path.dirname(require.resolve('eve'))] });"
        ),
        [
            "require ../other/x.js",
            "require require.resolve('eve')/../lib/y @resolved",
            "require eve",
        ]
    );
    // No options that matter: from the file.
    assert_eq!(
        refs(
            "require.resolve('a', undefined); require.resolve('b', {}); require.resolve('c', { \
             paths: undefined }); require.resolve('./d', { other: 1 });"
        ),
        [
            "require require.resolve('a') @resolved",
            "require require.resolve('b') @resolved",
            "require require.resolve('c') @resolved",
            "require ./d",
        ]
    );
    // Not understood: unknown, never the request from the file.
    for call in [
        "require.resolve('dep', { paths: [dir] })",
        "require.resolve('dep', opts)",
        "require.resolve('dep', { paths })",
        "require.resolve('dep', { paths: [process.cwd()] })",
        "require.resolve('dep', { paths: ['/abs'] })",
        "require.resolve('#imp', { paths: [require.resolve('eve')] }) || 0",
        "require.resolve('dep', ...rest)",
    ] {
        let out = refs(&format!(
            "module.exports = (dir, opts, paths, rest) => {call};"
        ));
        // The record shows the call (and, for a parameter, where the value comes from).
        let record = format!(
            "require <unknown> {{computed_specifier: {}",
            call.trim_end_matches(" || 0")
        );
        assert!(
            out.first().is_some_and(|r| r.starts_with(&record)),
            "{call}: {out:?}"
        );
        assert!(
            out[1..].iter().all(|r| r == "require eve"),
            "{call}: {out:?}"
        );
    }
    // The same through a `require` made by `createRequire(import.meta.url)`.
    assert_eq!(
        ft("import { createRequire } from 'node:module';\nconst req = \
            createRequire(import.meta.url);\nreq.resolve('dep', { paths: \
            [req.resolve('eve/package.json')] });")
        .into_iter()
        .skip(1)
        .collect::<Vec<_>>(),
        [
            "require require.resolve('dep', from require.resolve('eve/package.json')) \
             @Language(\"require.resolve\")",
            "require eve/package.json",
        ]
    );
}

/// `import.meta.resolve(x)` finds what `import(x)` would load.
#[test]
fn import_meta_resolve() {
    let refs = |body: &str| -> Vec<String> {
        ft(body)
            .into_iter()
            .map(|r| r.replace("@Language(\"require.resolve\")", "@resolved"))
            .collect()
    };
    assert_eq!(
        refs("const u = import.meta.resolve('eve'); const v = import.meta.resolve('./x.js');"),
        ["dynamic_import eve lazy", "dynamic_import ./x.js lazy"]
    );
    assert_eq!(
        refs(
            "const root = new URL('.', import.meta.resolve('eve'));\nawait import(new \
             URL('inner.js', root).href);"
        ),
        [
            "dynamic_import eve lazy",
            "dynamic_import import.meta.resolve('eve')/../inner.js @resolved lazy",
        ]
    );
    assert_eq!(
        refs("await import(new URL(p, new URL('.', import.meta.resolve('eve'))).href);"),
        [
            "dynamic_import import.meta.resolve('eve')/../* @resolved lazy {partial_specifier: p}",
            "dynamic_import eve lazy",
        ]
    );
    // A computed request is a site of its own; a parent URL is not followed.
    assert_eq!(
        refs("import.meta.resolve('./plugins/' + name);"),
        ["dynamic_import ./plugins/* lazy {partial_specifier: name}"]
    );
    assert_eq!(
        refs("import.meta.resolve('eve', parent);"),
        [
            "dynamic_import <unknown> lazy {computed_specifier: import.meta.resolve('eve', \
             parent)}"
        ]
    );
    assert_eq!(
        refs("import.meta.resolve('eve', undefined);"),
        ["dynamic_import import.meta.resolve('eve') @resolved lazy"]
    );
    // Not `import.meta` members that resolve nothing.
    assert_eq!(
        refs("const d = import.meta.dirname; import.meta.resolve;"),
        none()
    );
}

/// `vm` runs source text: code evaluation like `eval`, no file.
#[test]
fn vm_runs_code() {
    let diags = |src: &str| -> Vec<String> {
        scan(src.as_bytes(), Dialect::JS, ScanOptions::default())
            .diagnostics
            .iter()
            .map(|u| format!("{}: {}", u.kind, u.detail))
            .collect()
    };
    assert_eq!(
        diags(
            "const vm = require('node:vm');\nvm.runInNewContext(code, {}); vm.runInContext(a, \
             ctx); vm.runInThisContext(b);\nvm.compileFunction(body, ['x']); new vm.Script(src); \
             vm.createContext({});\nvm.isContext(x);"
        ),
        [
            "eval_like: vm.runInNewContext(code)",
            "eval_like: vm.runInContext(a)",
            "eval_like: vm.runInThisContext(b)",
            "eval_like: vm.compileFunction(body)",
            "eval_like: new vm.Script(src)",
        ]
    );
    assert_eq!(
        diags("import { runInNewContext as run, Script } from 'vm';\nrun(code); new Script(code);"),
        [
            "eval_like: vm.runInNewContext(code)",
            "eval_like: new vm.Script(code)"
        ]
    );
    assert_eq!(
        diags("require('vm').runInNewContext(code, sandbox);"),
        ["eval_like: vm.runInNewContext(code)"]
    );
    // Another object with the same method names.
    assert_eq!(
        diags("const vm = require('./my-vm'); vm.runInNewContext(code);"),
        none()
    );
}

#[test]
fn eval_like() {
    let ex = scan(
        b"eval('require(\"x\")'); new Function('a', 'return a'); obj.eval(1); require('y')",
        Dialect::JS,
        ScanOptions::default(),
    );
    assert_eq!(ex.refs.len(), 1);
    let d: Vec<_> = ex
        .diagnostics
        .iter()
        .map(|u| (u.kind, u.detail.to_string()))
        .collect();
    assert_eq!(
        d,
        [
            (
                UncertaintyKind::EvalLike,
                "eval('require(\"x\")')".to_string()
            ),
            (UncertaintyKind::EvalLike, "new Function('a')".to_string()),
        ]
    );
}

/// A `require` whose specifier cannot be said but ends with `.json` in every
/// alternative reads a data file: its open record is that of a read
/// (`fs_read`), not of a module load. What can be said stays a `require`
/// reference.
#[test]
fn computed_require_of_a_json_file_is_a_read() {
    // `kind spec {uncertainty kind}`.
    let k = |src: &str| -> Vec<String> {
        let ex = scan_named(
            src.as_bytes(),
            Dialect::TS,
            ScanOptions::default(),
            Some("main.ts"),
        );
        ex.refs
            .iter()
            .filter(|r| !matches!(r.kind, RefKind::Import))
            .map(|r| match &r.uncertainty {
                Some(u) => format!("{} {} {{{}}}", r.kind, r.spec, u.kind),
                None => format!("{} {}", r.kind, r.spec),
            })
            .collect()
    };
    let read = ["fs_read <unknown> {computed_specifier}"];
    let load = ["require <unknown> {computed_specifier}"];
    // A directory nothing in the file says, a literal file name.
    assert_eq!(
        k(
            "import { join } from 'node:path'; import { homedir } from 'node:os';\nconst dir = \
             join(homedir(), '.tool');\nexport const cfg = () => require(join(dir, 'auth.json'));"
        ),
        read
    );
    assert_eq!(
        k("export function load(dir: string) { return require(dir + '/config.json') }"),
        read
    );
    assert_eq!(
        k("export function load(dir: string) { return require(`${dir}/config.json`) }"),
        read
    );
    // Through a binding, and with a file name that is only partly literal: Node
    // goes by the extension.
    assert_eq!(
        k(
            "export function load(dir: string, name: string) {\nconst file = \
             `${dir}/${name}.json`;\n  return require(file);\n}"
        ),
        read
    );
    // Every alternative has to end with `.json`.
    assert_eq!(
        k(
            "export function load(dir: string, dev: boolean) {\nreturn require(dir + (dev ? \
             '/dev.json' : '/prod.json'));\n}"
        ),
        read
    );
    assert_eq!(
        k(
            "export function load(dir: string, dev: boolean) {\nreturn require(dir + (dev ? \
             '/dev.json' : '/prod.js'));\n}"
        ),
        load
    );
    // Not a `.json` file name: a module load as before.
    for src in [
        "export function load(dir: string) { return require(dir + '/config.js') }",
        "export function load(dir: string) { return require(dir + '/config') }",
        "export function load(dir: string) { return require(dir + '/.json') }",
        "export function load(dir: string) { return require(dir + '/config.json5') }",
        "export function load(name: string, ext: string) { return require(name + ext) }",
        "export function load(file: string) { return require(file) }",
    ] {
        assert_eq!(k(src), load, "{src}");
    }
    // What can be said is a `require` reference as before: a literal, an evaluated
    // literal, a known directory prefix (a membership).
    assert_eq!(k("require('./a.json')"), ["require ./a.json"]);
    assert_eq!(
        k("import { join } from 'node:path'; require(join(__dirname, 'data', 'a.json'))"),
        ["require ./data/a.json"]
    );
    assert_eq!(
        k("export function load(lang: string) { return require(`./locales/${lang}.json`) }"),
        ["require ./locales/*.json {partial_specifier}"]
    );
    // The other ways to call Node's `require`.
    assert_eq!(
        k("export function load(dir: string) { return module.require(dir + '/a.json') }"),
        read
    );
    assert_eq!(
        k("import { createRequire } from 'node:module';\nconst req = \
           createRequire(import.meta.url);\nexport function load(dir: string) { return req(dir + \
           '/a.json') }"),
        read
    );
    // `require.resolve` finds a file and reads nothing: what is done with the path
    // it returns is not known here.
    assert_eq!(
        k("export function dir(name: string) { return require.resolve(`${name}/package.json`) }"),
        load
    );
    assert_eq!(
        k("import { createRequire } from 'node:module';\nconst req = \
           createRequire(import.meta.url);\nexport function dir(name: string) { return \
           req.resolve(name + '/package.json') }"),
        load
    );
    // Other loaders are not concerned: `import()` evaluates under bundlers and
    // loaders, an alias of `require` is not known to be Node's.
    assert_eq!(
        k("export function load(dir: string) { return import(dir + '/config.json') }"),
        ["dynamic_import <unknown> {computed_specifier}"]
    );
    assert_eq!(
        k("const r = require; export function load(dir: string) { return r(dir + '/a.json') }"),
        ["require <unknown> {aliased_loader}"]
    );
    // The value has to be the value-flow engine's. A parameter that shadows a
    // constant is unknown to it; in a file it declines (`eval`), the token
    // evaluator's reading of the name (the constant, which ends with `.json`)
    // is a guess, and the record stays a load.
    let shadowed =
        "const p = base + '/a.json';\nexport function load(p: string) { return require(p) }";
    assert_eq!(k(shadowed), load);
    assert_eq!(k(&format!("eval('0');\n{shadowed}")), load);
    assert_eq!(
        k("eval('0');\nexport function load(dir: string) { return require(dir + '/a.json') }"),
        load
    );
    // A generated file of the temporary directory keeps its record.
    assert_eq!(
        k(
            "import os from 'node:os'; import path from 'node:path';\nexport const out = () => \
             require(path.join(os.tmpdir(), 'out.json'));"
        ),
        load
    );
}

/// `new Worker(code, { eval: true })` evaluates its first argument: code
/// evaluation, not a load of an unknown file. Only a literal `eval: true` that
/// nothing after it can replace says so.
#[test]
fn worker_with_eval_true_is_code_evaluation() {
    let scan_js = |src: &str| {
        let ex = scan_named(
            src.as_bytes(),
            Dialect::TS,
            ScanOptions::default(),
            Some("main.ts"),
        );
        let refs: Vec<String> = ex
            .refs
            .iter()
            .map(|r| match &r.uncertainty {
                Some(u) => format!("{} {} {{{}}}", r.kind, r.spec, u.kind),
                None => format!("{} {}", r.kind, r.spec),
            })
            .collect();
        let diags: Vec<String> = ex
            .diagnostics
            .iter()
            .map(|u| format!("{}: {}", u.kind, u.detail))
            .collect();
        (refs, diags)
    };
    let head = "import { Worker } from 'node:worker_threads';\nconst CODE = `postMessage(1)`;\n";
    let import = "import node:worker_threads";
    // The code is evaluated: no reference, one `eval_like` diagnostic.
    for (call, detail) in [
        (
            "new Worker(CODE, { eval: true, workerData: { a: 1 } });",
            "eval_like: new Worker(CODE, {eval: true})",
        ),
        (
            "new Worker(CODE, {\n  workerData: { eval: false },\n  'eval': true,\n});",
            "eval_like: new Worker(CODE, {eval: true})",
        ),
        (
            "new Worker(CODE, { ...opts, eval: true });",
            "eval_like: new Worker(CODE, {eval: true})",
        ),
        (
            "new Worker('require(\"./x\")', { eval: false, eval: true });",
            "eval_like: new Worker('require(\"./x\")', {eval: true})",
        ),
    ] {
        let (refs, diags) = scan_js(&format!("{head}{call}"));
        assert_eq!(refs, [import], "{call}");
        assert_eq!(diags, [detail], "{call}");
    }
    // A global `Worker` (no import) too.
    let (refs, diags) = scan_js("const CODE = 'x'; new Worker(CODE, { eval: true })");
    assert_eq!(refs, none());
    assert_eq!(diags, ["eval_like: new Worker(CODE, {eval: true})"]);
    // Not known to evaluate: the site stays a load of a file.
    for call in [
        "new Worker(CODE);",
        "new Worker(CODE, opts);",
        "new Worker(CODE, { eval: flag });",
        "new Worker(CODE, { eval });",
        "new Worker(CODE, { eval: false });",
        "new Worker(CODE, { eval: true, eval: false });",
        "new Worker(CODE, { eval: true, ...opts });",
        "new Worker(CODE, { eval: true, [key]: false });",
        "new Worker(CODE, { workerData: { eval: true } });",
        "new Worker(CODE, { eval: true && flag });",
    ] {
        let (refs, diags) = scan_js(&format!("{head}{call}"));
        assert_eq!(refs.len(), 2, "{call}: {refs:?}");
        assert!(refs[1].starts_with("worker "), "{call}: {refs:?}");
        assert_eq!(diags, none(), "{call}");
    }
    // A file path with other options is a load as before.
    let (refs, diags) =
        scan_js("new Worker(new URL('./w.js', import.meta.url), { eval: false, workerData: 1 })");
    assert_eq!(refs, ["worker w.js"]);
    assert_eq!(diags, none());
}

#[test]
fn ts_import_types_are_type_only() {
    let src = "type NodeFetch3RequestInit = \
               NotAny<import('../node_modules/node-fetch').RequestInit> | \
               NotAny<import('../../node_modules/node-fetch').RequestInit>;";
    assert_eq!(
        ft(src),
        [
            "dynamic_import ../node_modules/node-fetch lazy type_only",
            "dynamic_import ../../node_modules/node-fetch lazy type_only",
        ]
    );
    assert_eq!(
        ft(
            "type M = import('./m');\ntype U =\n  | import('./a')\n  | import('./b')\nconst x = \
             import('./runtime')"
        ),
        [
            "dynamic_import ./m lazy type_only",
            "dynamic_import ./a lazy type_only",
            "dynamic_import ./b lazy type_only",
            "dynamic_import ./runtime lazy",
        ]
    );
    assert_eq!(
        ft(
            "let a: import('./t1').T;\nfunction f(x: import('./t2').X): import('./t3') \
             {}\ninterface I { m: import('./t4').M }\ndeclare const d: import('./t5');\nconst g: \
             Foo<import('./t6')> = 1"
        ),
        [
            "dynamic_import ./t1 lazy type_only",
            "dynamic_import ./t2 lazy type_only",
            "dynamic_import ./t3 lazy type_only",
            "dynamic_import ./t4 lazy type_only",
            "dynamic_import ./t5 lazy type_only",
            "dynamic_import ./t6 lazy type_only",
        ]
    );
    // Runtime imports stay runtime.
    assert_eq!(
        ft(
            "const o = { a: import('./r1') };\nconst c = cond ? import('./r2') : \
             import('./r3');\nswitch (k) { case 1: import('./r4'); }\nconst m = await \
             import('./r5');\nimport('./r6').then(f);\nconst p: Promise<unknown> = import('./r7');"
        ),
        [
            "dynamic_import ./r1 lazy",
            "dynamic_import ./r2 lazy",
            "dynamic_import ./r3 lazy",
            "dynamic_import ./r4 lazy",
            "dynamic_import ./r5 lazy",
            "dynamic_import ./r6 lazy",
            "dynamic_import ./r7 lazy",
        ]
    );
    // JS has no type positions.
    assert_eq!(
        fj("x = import('./a').b; y = { a: import('./c') }"),
        ["dynamic_import ./a lazy", "dynamic_import ./c lazy"]
    );
}

#[test]
fn bindings_are_tracked_without_bindings_output() {
    let ex = scan(
        b"const fs = require('fs'); fs.readFileSync('/a')",
        Dialect::JS,
        ScanOptions::default(),
    );
    assert!(ex.bindings.is_empty());
    assert_eq!(ex.refs.len(), 2);
    assert_eq!(ex.refs[1].kind, RefKind::FsRead);
}

#[test]
fn template_with_leading_substitution() {
    assert_eq!(
        fj("const fs = require('fs'); fs.readFileSync(`${__dirname}/asset.txt`)"),
        ["require fs", "fs_read asset.txt"]
    );
    assert_eq!(fj("require(`${__dirname}/dep.js`)"), ["require ./dep.js"]);
    assert_eq!(
        fj(
            "import { readFileSync } from 'fs'; import { fileURLToPath } from 'url';
readFileSync(fileURLToPath(`${import.meta.url}/../${x ? 'a.txt' : 'b.txt'}`))"
        ),
        [
            "import fs",
            "import url",
            "fs_read a.txt | b.txt conditional"
        ]
    );
}

#[test]
fn deep_pass_assets() {
    // Assigned, returned, passed to an untracked call, read through a global `fs`.
    let src = "
const path = require('path');
let p = path.join(__dirname, '.', 'a.txt');
function f () { return __dirname + '/b.txt'; }
spawn('x', ['--y', path.join(__dirname, './c.txt')]);
fs.readFileSync(path.join(__dirname, 'assets', unknown) + '.txt');
fs.readFileSync(path.join(__dirname, 'more', ...rest) + '.txt');
const file = (0, path.join)(__dirname, 'd.txt');
x(cond ? path.join(__dirname, 'e.txt') : path.join(__dirname, 'f.txt'));
";
    assert_eq!(
        fj(src),
        [
            "require path",
            "asset a.txt",
            "asset b.txt",
            "asset c.txt",
            "asset assets/*.txt {partial_specifier: unknown}",
            "asset more/*.txt {partial_specifier: ...rest}",
            "asset d.txt",
            "asset e.txt",
            "asset f.txt",
        ]
    );
    // Never: the own directory, an ancestor, the file itself, a bare wildcard, or
    // an expression the main pass already resolved.
    let src = "
const path = require('path'); const fs = require('fs');
console.log(__dirname, path.join(__dirname, '..'), __filename, path.join(__dirname, x));
fs.readFileSync(path.join(__dirname, 'g.txt'));
";
    assert_eq!(fj(src), ["require path", "require fs", "fs_read g.txt"]);
    // process.cwd(), and a webpack namespace object taken as `path` (with a
    // record).
    let src = "
const p = X.a.join(process.cwd(), 'assets', 'dict.json');
o.resolve(__dirname, './h.txt');
";
    assert_eq!(
        fj(src),
        [
            "asset assets/dict.json @cwd {cwd_dependent: X.a.join(process.cwd(), 'assets', \
             'dict.json'); `X.a.join(process.cwd(), 'assets', 'dict.json')` assumed to be \
             `path.join`}",
            "asset h.txt {partial_specifier: `o.resolve(__dirname, './h.txt')` assumed to be \
             `path.resolve`}",
        ]
    );
    // Top-level names holding paths are anchors too.
    let src = "
var path = require('path');
var dir = path.resolve(__dirname, '.', '.');
var bin = os.platform() === 'win32' ? 'a.exe' : 'a.exe';
var full = path.join(dir, bin);
";
    assert_eq!(
        fj(src),
        [
            "require path",
            "asset a.exe {partial_specifier: `dir` is a `let`/`var` or function-local binding; \
             its initial value is used (later assignments are not tracked)}",
        ]
    );
}

#[test]
fn deep_pass_resolve_from() {
    let src = "
var resolveFrom = require('resolve-from');
var x = resolveFrom(__dirname, './dep.js');
require(x);
";
    assert_eq!(
        fj(src),
        ["require resolve-from", "asset dep.js", "require ./dep.js",]
    );
}

#[test]
fn deep_pass_require_wrappers() {
    for src in [
        "const reaction = name => { const res = require(name); return res; }; reaction('./dep');",
        "function reaction (name) { return require(name); } reaction('./dep');",
        "const reaction = (a, name) => { return require(name); }; reaction(1, './dep');",
        "const reaction = name => require(name); reaction('./dep');",
    ] {
        // Every call of the wrapper is in the file: the value-flow pass binds its
        // parameter.
        assert_eq!(fj(src), ["require ./dep"], "{src}");
    }
    assert_eq!(
        fj("function f (a) { const b = a + 1; return require(b); } f('./x');"),
        ["require ./x1"]
    );
    // An exported wrapper can be called from elsewhere: the call in the file is a
    // reference, the site itself stays open (and the wrapper rule still applies).
    assert_eq!(
        fj("function load (name) { return require(name); } exports.load = load; load('./dep');"),
        [
            "require ./dep conditional",
            "require <unknown> conditional {computed_specifier: name; parameter load#0}",
            "require ./dep {aliased_loader: `load` passes its parameter to `require` (require \
             wrapper)}",
        ]
    );
}

#[test]
fn deep_pass_browserify() {
    let src = r#"
!function(e){}(function(){ var i = "function" == typeof require && require;
var n = new Error("x"); n.code = "MODULE_NOT_FOUND";
return r({
  1: [function (s, e, t) { var a = s("acorn"); var b = s("./local"); }, { acorn: void 0, "./local": 2 }],
  2: [function (s, e, t) {}, {}]
}, {}, [1]) });
"#;
    assert_eq!(
        fj(src),
        [
            "require acorn {aliased_loader: browserify bundle: `s` is the module's `require` and \
             the dependency is not bundled}"
        ]
    );
}

#[test]
fn deep_pass_pino_targets() {
    let src = "
const pino = require('pino');
pino.transport({ targets: [{ target: 'transport-a', level: 'error' }, { target: './t.js' }] });
";
    assert_eq!(
        fj(src),
        [
            "require pino",
            "require transport-a optional {aliased_loader: pino transport `target`: loaded by \
             pino in a worker thread}",
            "require ./t.js optional {aliased_loader: pino transport `target`: loaded by pino in \
             a worker thread}",
        ]
    );
    // Without pino or fastify, `target` is just a property.
    assert_eq!(
        fj("module.exports = { target: 'es2020' }"),
        Vec::<String>::new()
    );
}

#[test]
fn deep_pass_function_local_anchors() {
    let src = "
const path = require('path');
function f() {
  const Se = require('url').pathToFileURL(__filename).href;
  return j.readFileSync(new URL('./llhttp/llhttp.wasm', Se));
}
const g = () => {
  const root = path.join(__dirname, '..');
  return path.join(root, 'tpl', 'x.html');
};
function h() { return path.join(root, 'out-of-scope.html'); }
const Q = require('url').pathToFileURL(__filename).href;
k(new URL('./top.wasm', Q));
";
    assert_eq!(
        fj(src),
        [
            "require path",
            "require url",
            "asset llhttp/llhttp.wasm {partial_specifier: `Se` is a `let`/`var` or function-local \
             binding; its initial value is used (later assignments are not tracked)}",
            "asset ../tpl/x.html {partial_specifier: `root` is a `let`/`var` or function-local \
             binding; its initial value is used (later assignments are not tracked)}",
            "require url",
            "fs_read top.wasm",
        ]
    );
}

/// P11 precondition: a cwd-relative require keeps a literal tail with base
/// `cwd` and only a `cwd_dependent` uncertainty (no `computed_specifier`), so a
/// test context (jest's working directory, `Project::test_cwds`) can resolve
/// it.
#[test]
fn cwd_relative_requires_keep_a_literal_tail() {
    for src in [
        "require(process.cwd() + '/src/routes')",
        "require(require('path').join(process.cwd(), 'src', 'routes'))",
        "const p = process.cwd() + '/src/routes'; require(p)",
    ] {
        let got = fj(src);
        let r = got
            .iter()
            .find(|s| s.starts_with("require ./src/routes"))
            .unwrap_or_else(|| panic!("{src}: {got:?}"));
        assert!(r.contains(" @cwd"), "{r}");
        assert!(r.contains("{cwd_dependent:"), "{r}");
        assert!(
            !got.iter().any(|s| s.contains("computed_specifier")),
            "{got:?}"
        );
    }
}

#[test]
fn module_strings_in_registry_positions() {
    let ms =
        |spec: &str| format!("dynamic_import {spec} @Language(\"module_string\") optional lazy");
    // Object property values and array elements that look like module paths.
    assert_eq!(
        full(
            "export const routes = [\n  { method: 'GET', path: '/v1/x', handler: \
             'services/a/src/handlers/get' },\n];",
            Dialect::TS
        ),
        [ms("services/a/src/handlers/get")]
    );
    assert_eq!(
        full(
            "module.exports = { stubs: ['./lib/stub.ts', `../shared/util`, 'application/json'] };",
            Dialect::JS
        ),
        [
            ms("./lib/stub.ts"),
            ms("../shared/util"),
            ms("application/json")
        ]
    );
    // Once per distinct string.
    assert_eq!(
        full("const t = { a: 'x/y', b: 'x/y' };", Dialect::JS),
        [ms("x/y")]
    );
    // Not registry positions: call arguments, plain values, member accesses.
    for src in [
        "f('a/b'); g(1, 'a/b');",
        "const x = 'a/b'; let y = c ? 'a/b' : 'c/d';",
        "const v = table['a/b']; const w = f()['a/b'];",
        "type T = 'a/b' | 'c/d';",
        "// { k: 'a/b' }\n/* ['a/b'] */",
    ] {
        assert_eq!(full(src, Dialect::TS), none(), "{src}");
    }
    // Not module paths: absolute paths, URLs, package specifiers, one segment, no
    // letter.
    for src in [
        "const t = { a: '/v1/teams', b: 'https://x.dev/y', c: '@scope/pkg/x', d: 'single' };",
        "const t = ['a//b', 'a/b/', '1/2', 'a b/c', 'a/./b', '../..'];",
    ] {
        assert_eq!(full(src, Dialect::JS), none(), "{src}");
    }
    // Specifiers of real references are not repeated.
    assert_eq!(
        full("const m = { load: require('./a/b') };", Dialect::JS),
        ["require ./a/b"]
    );
    // Declaration files have no runtime strings.
    assert_eq!(full("declare const t: { a: 'x/y' };", Dialect::DTS), none());
}

#[test]
fn module_string_shapes() {
    for ok in [
        "a/b",
        "services/svc-a-teams/src/routes.ts",
        "./x",
        "../../lib/y.js",
        "src/a@2x",
    ] {
        assert!(module_string_shape(ok.as_bytes()), "{ok}");
    }
    for bad in [
        "a", "/a/b", "a/b/", "a//b", "@a/b", "-a/b", "a/b c", "a/../b", "./", "..", "1/2", "a/b?x",
    ] {
        assert!(!module_string_shape(bad.as_bytes()), "{bad}");
    }
}
