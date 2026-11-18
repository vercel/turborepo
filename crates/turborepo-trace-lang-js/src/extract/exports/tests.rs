use std::time::{Duration, Instant};

use super::*;

fn ts(src: &str) -> Shape<'_> {
    scan_shape(src.as_bytes(), Dialect::TS)
}

fn js(src: &str) -> Shape<'_> {
    scan_shape(src.as_bytes(), Dialect::JS)
}

fn names<'a>(v: &[&'a str]) -> Names<'a> {
    Names::Some(v.to_vec())
}

fn use_of<'a>(s: &Shape<'a>, spec: &str) -> Names<'a> {
    let us: Vec<_> = s.uses.iter().filter(|u| u.spec == spec).collect();
    assert_eq!(us.len(), 1, "one use of {spec}: {:?}", s.uses);
    us[0].names.clone()
}

fn sef(src: &str) -> bool {
    let s = ts(src);
    assert!(s.understood, "not understood: {src}");
    s.side_effect_free
}

// ---- uses -----------------------------------------------------------------------------

#[test]
fn named_default_and_type_specifiers() {
    let s = ts("import a, { b, c as d, type E, default as f, 'g h' as g } from './x';\n");
    assert_eq!(use_of(&s, "./x"), names(&["E", "b", "c", "default", "g h"]));
    assert!(!s.uses[0].type_only);
    assert_eq!(s.uses[0].start, 0);
    assert_eq!(s.executes, ["./x"]);
}

#[test]
fn use_start_is_the_import_keyword() {
    let src = "// c\n  import { a } from './a';\nconst x = 1; const { b } = require('./b');\n";
    let s = ts(src);
    assert_eq!(s.uses[0].start as usize, src.find("import").unwrap());
    assert_eq!(s.uses[1].start as usize, src.find("require").unwrap());
}

#[test]
fn type_imports() {
    let s = ts(
        "import type { T } from './t';\nimport type D from './d';\nimport type * as N from \
         './n';\n",
    );
    assert_eq!(use_of(&s, "./t"), names(&["T"]));
    assert!(s.uses.iter().all(|u| u.type_only));
    assert_eq!(use_of(&s, "./d"), names(&["default"]));
    assert!(s.executes.is_empty());
    // `import type from` is a default import named `type`.
    let s = ts("import type from './x';\n");
    assert_eq!(use_of(&s, "./x"), names(&["default"]));
    assert!(!s.uses[0].type_only);
    // `import {type}` imports the export named `type`; `{type as t}` too.
    let s = ts("import { type, type as t2 } from './x';\n");
    assert_eq!(use_of(&s, "./x"), names(&["type"]));
}

#[test]
fn side_effect_import_uses_nothing() {
    let s = ts("import './polyfill';\n");
    assert_eq!(use_of(&s, "./polyfill"), names(&[]));
    assert_eq!(s.executes, ["./polyfill"]);
}

#[test]
fn namespace_with_static_members() {
    let s = ts("import * as ns from './x';\nns.a(); const v = ns.b; f(ns?.c.d); type T = ns.D;\n");
    assert_eq!(use_of(&s, "./x"), names(&["D", "a", "b", "c"]));
}

#[test]
fn namespace_used_as_value_uses_everything() {
    for src in [
        "import * as ns from './x';\nf(ns);\n",
        "import * as ns from './x';\nconst { a } = ns;\n",
        "import * as ns from './x';\nns['a'];\n",
        "import * as ns from './x';\nexport { ns };\n",
        "import * as ns from './x';\ntype T = typeof ns;\n",
        "import d, * as ns from './x';\nconsole.log(ns);\n",
    ] {
        assert_eq!(use_of(&ts(src), "./x"), Names::All, "{src}");
    }
    // A property named like the namespace is not a use.
    let s = ts("import * as ns from './x';\nobj.ns(); ns.a;\n");
    assert_eq!(use_of(&s, "./x"), names(&["a"]));
}

#[test]
fn namespace_in_jsx_uses_everything() {
    let s = scan_shape(
        b"import * as Icons from './icons';\nexport const A = () => <Icons.Star />;\n",
        Dialect::TSX,
    );
    assert_eq!(use_of(&s, "./icons"), Names::All);
}

#[test]
fn import_equals_require_uses_everything() {
    let src = "import x = require('./x');\nimport type y = require('./y');\n";
    let s = ts(src);
    assert_eq!(use_of(&s, "./x"), Names::All);
    assert_eq!(s.uses[0].start, 0);
    assert!(s.uses[1].type_only);
    assert_eq!(s.executes, ["./x"]);
}

#[test]
fn require_shapes() {
    let s = js("const { a, b: c, 'd': e } = require('./x');\n");
    assert_eq!(use_of(&s, "./x"), names(&["a", "b", "d"]));
    let s = js("const { a, ...rest } = require('./x');\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = js("const { [k]: v } = require('./x');\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = js("const m = require('./x');\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = js("const f = require('./x').foo;\n");
    assert_eq!(use_of(&s, "./x"), names(&["foo"]));
    let s = js("require('./x')['foo'];\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = js("require('./x')();\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = js("function f() { const { a } = require('./x'); }\n");
    assert_eq!(use_of(&s, "./x"), names(&["a"]));
}

#[test]
fn unrecognized_references_are_not_listed() {
    // Callers treat references without an entry as using everything.
    let s = js("import('./a'); require.resolve('./b'); module.require('./c'); require(x);\n");
    assert!(s.uses.is_empty(), "{:?}", s.uses);
}

#[test]
fn mocks_use_everything() {
    let s =
        ts("import { a } from './x';\nimport { b } from './y';\njest.mock('./x', () => ({}));\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    assert_eq!(use_of(&s, "./y"), names(&["b"]));
    assert_eq!(s.mocked, ["./x"]);
    assert!(!s.uses_all);
    let s = ts("import { a } from './x';\nvi.mock(import('./x'));\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = ts("import { a } from './x';\njest.requireActual('./x');\n");
    assert_eq!(use_of(&s, "./x"), Names::All);
    let s = ts("import { a } from './x';\njest.mock(name);\n");
    assert!(s.uses_all);
    // Not a mock: a member of something else.
    let s = ts("import { a } from './x';\nfoo.jest.mock('./x');\n");
    assert_eq!(use_of(&s, "./x"), names(&["a"]));
}

#[test]
fn escaped_specifiers_are_not_listed() {
    let s = ts("import { a } from './\\x61';\n");
    assert!(s.uses.is_empty());
}

// ---- re-exports and exports
// -----------------------------------------------------------

#[test]
fn reexports() {
    let src = "export * from './a';\nexport * as ns from './b';\nexport { x, y as z, default, \
               default as d } from './c';\nexport type { T } from './t';\nexport type * from \
               './u';\n";
    let s = ts(src);
    assert_eq!(s.reexports.len(), 5);
    assert_eq!(s.reexports[0].kind, ReExportKind::Star);
    assert_eq!(s.reexports[0].start, 0);
    assert_eq!(s.reexports[1].kind, ReExportKind::Namespace("ns"));
    assert_eq!(
        s.reexports[2].kind,
        ReExportKind::Named(vec![
            ("x", "x"),
            ("y", "z"),
            ("default", "default"),
            ("default", "d")
        ])
    );
    assert!(s.reexports[3].type_only);
    assert_eq!(s.reexports[3].kind, ReExportKind::Named(vec![("T", "T")]));
    assert!(s.reexports[4].type_only && s.reexports[4].kind == ReExportKind::Star);
    assert_eq!(s.executes, ["./a", "./b", "./c"]);
    // `export *` makes the file's own export set open.
    assert!(s.exports_open);
    assert!(!ts("export * as ns from './b';\n").exports_open);
}

#[test]
fn local_exports() {
    let src = "export const a = 1, b = 2;\nexport let c: Map<string, number> = new Map(), \
               d;\nexport function f() {}\nexport async function af() {}\nexport function* g() \
               {}\nexport class C {}\nexport abstract class AC {}\nexport interface I {}\nexport \
               type T = 1;\nexport enum E { A }\nexport const enum F { B }\nexport declare const \
               q: number;\nexport namespace N {}\nconst z = 1;\nexport { z as y, z };\nexport \
               default 1;\n";
    let s = ts(src);
    assert_eq!(
        s.exports,
        [
            "AC", "C", "E", "F", "I", "N", "T", "a", "af", "b", "c", "d", "default", "f", "g", "q",
            "y", "z"
        ]
    );
    assert!(!s.exports_open, "{s:?}");
    assert!(s.module_syntax);
}

#[test]
fn open_export_sets() {
    for src in [
        "module.exports = { a: 1 };\n",
        "exports.a = 1;\n",
        "Object.defineProperty(exports, 'a', { value: 1 });\n",
        "export = foo;\n",
        "export const { a, b } = obj;\n",
        "export * from './x';\n",
    ] {
        assert!(ts(src).exports_open, "{src}");
    }
}

#[test]
fn declarator_names_are_cross_checked() {
    // `a < b, c > d` is a comparison, not type arguments: `y` must still be an
    // export.
    let s = ts("export const x = 1 < 2, y = 3 > 2;\n");
    assert!(s.side_effect_free);
    assert!(s.exports_open || s.exports.contains(&"y"), "{s:?}");
}

#[test]
fn executes_skips_type_only_loads() {
    let s = ts(
        "import type { A } from './t';\nimport { type B } from './b';\nexport type { X } from \
         './x';\nimport './d';\nexport { y } from './y';\n",
    );
    assert_eq!(s.executes, ["./b", "./d", "./y"]);
}

#[test]
fn scripts_have_no_module_syntax() {
    let s = js("function f() {}\n");
    assert!(!s.module_syntax);
    assert!(s.side_effect_free);
}

// ---- side effects
// ---------------------------------------------------------------------

#[test]
fn side_effect_free_top_levels() {
    for src in [
        "",
        "export function f() { sideEffect(); }\n",
        "export const a = 1, b = 'x', c = `t`, d = true, e = null, f = undefined, g = -1, h = \
         !0;\n",
        "export const g = () => console.log('x');\n",
        "export const h = async () => { await x(); };\n",
        "export const i = function named() { return 1 };\n",
        "export const j = { a: 1, b() { return 2 }, get c() { return 3 }, 'd': [1, 2, { e: 3 }], \
         4: null };\n",
        "const base = 'x';\nexport const url = `${base}/y`;\n",
        "const MIN = 60;\nexport const HOUR = MIN * 60, DAY = HOUR * 24;\n",
        "export class A { x = foo(); static y = 1; m() { bar() } static s() {} \
         constructor(private z: number) {} }\n",
        "class B extends Error { constructor(m: string) { super(m) } }\nexport class C extends B \
         {}\n",
        "export enum E { A, B = 'b', C = -1, D = 2 }\n",
        "type T = { a: string } | null;\ninterface I extends J<{ x: 1 }> { a: T }\n",
        "declare const x: number;\ndeclare function f(a: string): void;\n",
        "import { a } from './a';\nexport * from './b';\nexport { a };\n",
        "'use strict';\nexport default function () {}\n",
        "export default { a: 1 };\n",
        "export default class {}\n",
        "export const x = 1 as const;\nexport const y = { a: 1 } satisfies Foo;\n",
        "export const fn = <T,>(x: T): T => x;\n",
        "export function over(a: string): void;\nexport function over(a: any) {}\n",
        "export const handlers = { a, b: a };\nfunction a() {}\n",
        "export const re = /x/g;\n",
        "let x: Map<string, number> | undefined;\n",
        "export const typed: Record<string, Array<number>> = {};\n",
        "export const cond = 1 > 0 ? 'a' : 'b';\n",
        "export type Fn = <T>(x: T) => T;\n",
        "export const a = 1\nexport const b = 2\n",
        "type A =\n  | 'a'\n  | 'b'\nexport const x = 1\n",
        "export type Cond<T> = T extends string ? 'a' : T extends number ? 'b' : never;\n",
        "export const f = (x: number): Promise<void> => g(x);\nexport const k = 2;\n",
        "export function r(): { a: number } { return { a: 1 }; }\n",
        "export abstract class Base { abstract run(): void; protected x?: string; }\n",
        "export const enum K { A = 1 }\n",
        "import x = require('./x');\nexport const y = 1;\n",
        "export const obj = { async m() {}, *gen() {}, get g() { return 1 }, set g(v) {} };\n",
        "export const arr = [1, , 2];\n",
        "export const f2 = async x => x;\n",
        "export const s = 'a' + 'b', n = 1 << 2 | 1, cmp = 2 >= 1;\n",
        "export const v = void 0, t = typeof 1;\n",
    ] {
        assert!(sef(src), "expected side-effect free: {src}");
    }
}

#[test]
fn side_effect_free_jsx_definitions() {
    let s = scan_shape(
        b"import React from 'react';\nexport const C = () => <div>{x}</div>;\nexport function D() { return <span />; }\n",
        Dialect::TSX,
    );
    assert!(s.understood && s.side_effect_free, "{s:?}");
}

#[test]
fn top_levels_with_side_effects() {
    for src in [
        "foo();\n",
        "export const a = foo();\n",
        "export const a = new Map();\n",
        "export const a = obj.prop;\n",
        "import { y } from './y';\nexport const x = y;\n",
        "import { y } from './y';\nexport const x = { y };\n",
        "export const a = { ...b };\n",
        "export const a = [...b];\n",
        "export const a = { [k]: 1 };\n",
        "const later = after;\nconst after = 1;\n",
        "export const a = b, b = 1;\n",
        "export class A { static x = foo(); }\n",
        "export class A { static { foo() } }\n",
        "@dec\nexport class A {}\n",
        "import { Base } from './base';\nexport class A extends Base {}\n",
        "export class A extends mixin(B) {}\n",
        "class A { [k] = 1 }\n",
        "class A { [key: string]: number }\n",
        "export const a = 1\n(foo)()\n",
        "export const f = () => 1\nfoo()\n",
        "type A = B\nfoo()\n",
        "declare global { interface Window { a: 1 } }\n",
        "declare module 'x' { export const a: 1 }\n",
        "namespace N { export const a = 1 }\n",
        "await x;\n",
        "export const a = await x;\n",
        "export default foo();\n",
        "export const t = tag`x`;\n",
        "export const a = `${obj}`;\n",
        "if (x) {}\n",
        "export const a = 1n + 1;\n",
        "module.exports = {};\n",
        "exports.a = 1;\n",
        "export enum E { A = foo() }\n",
        "export enum E { A = 1 << 2 }\n",
        "let x = 1;\nx = 2;\n",
        "const x = 1;\nconst x = 2;\n",
        "export const a = this;\n",
        "export const a = process.env.X;\n",
        "export const a = globalThis;\n",
        "import.meta.hot;\n",
        "export const a = import('./x');\n",
        "export const a = require('./x');\n",
        "const { a } = obj;\n",
        "export = {};\n",
        "label: for (;;) {}\n",
        "export const a = 1, b = a.c;\n",
        "export const a = {} + 1;\n",
        "export const fn = function () {}\n()\n",
        "export const a = 1;\nexport { a } from './a' + x;\n",
    ] {
        assert!(!sef(src), "expected side effects: {src}");
    }
}

#[test]
fn jsx_values_have_side_effects() {
    for src in [
        "export const el = <div />;\n",
        "const x = 1;\nexport const el = <div>{x}</div>;\n",
        "const x = 1;\nexport const el = <div>{x}\n</div>;\n",
        "const x = 1;\nexport const el = <div a={{ x }} />;\n",
    ] {
        let s = scan_shape(src.as_bytes(), Dialect::TSX);
        assert!(!s.side_effect_free, "{src}");
    }
}

#[test]
fn malformed_input_is_not_understood() {
    for src in [
        "export const a = 'unterminated\n",
        "export const a = (1;\n",
        "}}}\n",
        "`${\n",
    ] {
        let s = ts(src);
        assert!(!s.understood && !s.side_effect_free, "{src}");
        assert!(s.uses.is_empty() && s.reexports.is_empty() && s.exports.is_empty());
    }
}

#[test]
fn deep_nesting_does_not_overflow() {
    let n = 200_000;
    let src = format!("export const a = {}1{};\n", "[".repeat(n), "]".repeat(n));
    let s = ts(&src);
    assert!(!s.side_effect_free);
    let src = format!("export const a = {}1;\n", "-".repeat(n));
    let s = ts(&src);
    assert!(!s.side_effect_free);
    let src = format!("export const a = {}1{};\n", "(".repeat(n), ")".repeat(n));
    assert!(!ts(&src).side_effect_free);
}

#[test]
fn arbitrary_bytes_never_panic() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let alphabet: &[u8] = b"import export from as type * { } ( ) [ ] < > = => , ; : ? . ... ' \" ` ${ / \n const let class extends static function async await new require jest.mock x y 1 n @ # ! - + | &";
    for _ in 0..2000 {
        let mut src = Vec::new();
        for _ in 0..64 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let a = (state as usize) % alphabet.len();
            let len = 1 + (state >> 32) as usize % 6;
            src.extend_from_slice(&alphabet[a..(a + len).min(alphabet.len())]);
        }
        for d in [
            Dialect::JS,
            Dialect::TS,
            Dialect::TSX,
            Dialect::JSX,
            Dialect::DTS,
        ] {
            let _ = scan_shape(&src, d);
        }
    }
    let _ = scan_shape(&[0xff, 0xfe, b'\'', 0x80], Dialect::JS);
}

#[test]
fn deterministic() {
    let src = b"import * as a from './a';\nimport { b } from './b';\nexport * from './c';\nexport const x = 1;\na.y; a.z;\n";
    assert_eq!(scan_shape(src, Dialect::TS), scan_shape(src, Dialect::TS));
}

/// Best of `n` timings of a scan.
fn best_of(src: &[u8], n: usize) -> Duration {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(scan_shape(std::hint::black_box(src), Dialect::TS));
            t.elapsed()
        })
        .min()
        .unwrap_or_default()
}

#[test]
fn scan_is_linear() {
    let units: &[&str] = &[
        "import * as ns from './x';\nns.a; ns.b(ns.c);\n",
        "import * as ns from './x';\n",
        "export const a1 = { b: [1, 2, { c: () => d }], e() {} };\n",
        "const { a, b } = require('./x'); require('./y').z;\n",
        "export * from './a';\nexport { b as c } from './b';\n",
        "type T<A> = A extends Array<infer U> ? U : never;\n",
        "class K { x = 1; static m() {} }\n",
        "jest.mock('./m');\nimport { q } from './m';\n",
        "export const s = `a${1}b`;\n",
        "<<<<",
        "((((",
        "export const x = 1 < 2, y = 3 > 2;\n",
    ];
    for unit in units {
        let mut n = 64;
        let mut small = unit.repeat(n);
        while best_of(small.as_bytes(), 3) < Duration::from_millis(2) && small.len() < 4 << 20 {
            n *= 2;
            small = unit.repeat(n);
        }
        let big = unit.repeat(8 * n);
        let mut ratio = f64::INFINITY;
        for _ in 0..8 {
            let s = best_of(small.as_bytes(), 2).as_secs_f64().max(1e-9);
            let b = best_of(big.as_bytes(), 1).as_secs_f64();
            ratio = ratio.min(b / s);
            if ratio <= 24.0 {
                break;
            }
        }
        assert!(ratio <= 24.0, "super-linear on {unit:?}: ratio {ratio:.1}");
    }
}

/// Dumps shapes of the files listed in `$SHAPE_CORPUS` (paths relative to
/// `$SHAPE_ROOT`) as JSON lines on stdout, for differential checks against a
/// full parser on real code.
#[test]
#[ignore = "corpus dump; needs SHAPE_CORPUS and SHAPE_ROOT"]
fn shape_corpus_dump() {
    let (Ok(list), Ok(root)) = (std::env::var("SHAPE_CORPUS"), std::env::var("SHAPE_ROOT")) else {
        return;
    };
    let list = std::fs::read_to_string(list).unwrap_or_default();
    for path in list.lines() {
        let Ok(src) = std::fs::read(std::path::Path::new(&root).join(path)) else {
            continue;
        };
        let s = scan_shape(&src, Dialect::from_path(std::path::Path::new(path)));
        let uses: Vec<serde_json::Value> = s
            .uses
            .iter()
            .map(|u| {
                serde_json::json!({
                    "start": u.start,
                    "spec": u.spec,
                    "names": match &u.names { Names::All => None, Names::Some(v) => Some(v) },
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "path": path,
                "understood": s.understood,
                "sef": s.side_effect_free,
                "open": s.exports_open,
                "exports": s.exports,
                "uses_all": s.uses_all,
                "uses": uses,
            })
        );
    }
}
