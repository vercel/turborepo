//! Regression tests of the soundness review of the value-flow engine.
//!
//! Every case is a program whose runtime values were observed with the flow
//! oracle (`crates/turborepo-trace-lang-js/tests/flow_oracle`): the
//! engine claimed a value set that did not contain what the program reads. The
//! expectation is always that the references cover the runtime values, or that
//! the site keeps a record.

use super::tests::{show, show_as};
use crate::lexer::Dialect;

// ---- numbers and booleans ----

#[test]
fn arithmetic_on_numbers_is_not_concatenation() {
    // `n + 1` is 2, not "11".
    assert_eq!(
        show("const n = 1;\nreadFileSync(join(__dirname, 'f' + (n + 1) + '.json'));\n"),
        "fs_read dir:f*.json !partial_specifier/unknown"
    );
    // A loop counter plus one.
    assert_eq!(
        show(
            "for (let i = 0; i < 3; i++) {\n  \
             const p = join(__dirname, `chunk-${i + 1}.json`);\n  readFileSync(p);\n}\n"
        ),
        "fs_read dir:chunk-*.json !partial_specifier/unknown"
    );
    // `+=` on a number adds.
    assert_eq!(
        show(
            "let c = 0;\nc += 1;\nconst p = join(__dirname, \
             `count-${c}.json`);\nreadFileSync(p);\n"
        ),
        "fs_read dir:count-0.json | count-*.json !partial_specifier/unknown"
    );
    // An index computed from numbers selects any element.
    assert_eq!(
        show(
            "const table = ['zero.json', 'one.json', 'two.json'];\nconst one = 1;\nconst p = \
             join(__dirname, table[one + 1]);\nreadFileSync(p);\n"
        ),
        "fs_read dir:zero.json | one.json | two.json"
    );
    // Digits next to text are still text.
    assert_eq!(
        show("const v = 2;\nconst p = join(__dirname, 'v' + v, 'x' + 1 + 2);\nreadFileSync(p);\n"),
        "fs_read dir:v2/x12"
    );
}

#[test]
fn lengths_indexes_and_counters_are_some_number() {
    assert_eq!(
        show(
            "const xs = ['a', 'b'];\nconst a = join(__dirname, 'data-' + xs.length + \
             '.json');\nreadFileSync(a);\nconst b = join(__dirname, 'idx' + \
             'abc'.indexOf('c'));\nreadFileSync(b);\nconst c = join(__dirname, \
             `len-${xs.push('z')}`);\nreadFileSync(c);\nconst m = new Map([['k', 'v']]);\nconst d \
             = join(__dirname, 'size' + m.size);\nreadFileSync(d);\n"
        ),
        "fs_read dir:data-*.json !partial_specifier/unknown\nfs_read dir:idx* \
         !partial_specifier/unknown\nfs_read dir:len-* !partial_specifier/unknown\nfs_read \
         dir:size* !partial_specifier/unknown"
    );
    // `x.n++` changes the property.
    assert_eq!(
        show(
            "const counters = { n: 0 };\ncounters.n++;\nconst p = join(__dirname, \
             `o-${counters.n}.json`);\nreadFileSync(p);\n"
        ),
        "fs_read dir:o-0.json | o-*.json !partial_specifier/unknown"
    );
    // Also in a function the object is passed to.
    assert_eq!(
        show(
            "const counted = { n: 0 };\nfunction bump(o: { n: number }) { o.n++; \
             }\nbump(counted);\nconst p = join(__dirname, 'counted-' + \
             counted.n);\nreadFileSync(p);\n"
        ),
        "fs_read dir:counted-0 | counted-* !partial_specifier/unknown"
    );
}

#[test]
fn booleans_are_text_in_a_name_and_keys_of_a_table() {
    // A table of booleans.
    assert_eq!(
        show(
            "describe.each([true, false])('minify %s', (minify) => {\n  const p = join(__dirname, \
             `out-${minify}.js`);\n  readFileSync(p);\n});\n"
        ),
        "fs_read dir:out-*.js !partial_specifier/unknown"
    );
    // A comparison as a key.
    assert_eq!(
        show(
            "const t = { true: 'yes.json', false: 'no.json' };\nexport function f(a: string) {\n  \
             const p = join(__dirname, t[a === 'x']);\n  readFileSync(p);\n  const q = \
             join(__dirname, t[String(a === 'x')]);\n  readFileSync(q);\n}\n"
        ),
        "fs_read dir:yes.json | no.json\nfs_read dir:yes.json | no.json"
    );
    // `String(flag)`, `!flag`, `flag.toString()` inside a name.
    assert_eq!(
        show(
            "export function f(a: string) {\n  const flag = a === 'x';\n  const p = \
             join(__dirname, String(flag) + '.json');\n  readFileSync(p);\n  const q = \
             join(__dirname, `${!flag}.txt`);\n  readFileSync(q);\n}\n"
        ),
        "fs_read dir:./*.json !partial_specifier/unknown\nfs_read dir:./*.txt \
         !partial_specifier/unknown"
    );
    // By itself a boolean is still no path: no reference and no record.
    assert_eq!(
        show(
            "export function f(a: string) {\n  const p = a === 'x' ? join(__dirname, 'a.json') : \
             false;\n  readFileSync(p);\n}\n"
        ),
        "fs_read dir:a.json"
    );
}

#[test]
fn undefined_and_null_as_keys_and_in_a_joined_array() {
    assert_eq!(
        show(
            "const u = { undefined: 'u.json', null: 'n.json', other: 'o.json' };\nlet key;\nconst \
             p = join(__dirname, u[key]);\nreadFileSync(p);\n"
        ),
        "fs_read dir:u.json | n.json"
    );
    // `[a, undefined, b].join('/')` is `a//b`.
    assert_eq!(
        show(
            "export function f(deep: boolean) {\n  const p = [__dirname, 'fx', deep ? 'sub' : \
             undefined, 'c.json'].join('/');\n  readFileSync(p);\n}\n"
        ),
        "fs_read dir:fx/sub/c.json | fx/c.json"
    );
}

#[test]
fn a_falsy_left_operand_of_and_is_a_value() {
    // `(cond && a) || b` is `b` when `cond` is false.
    assert_eq!(
        show(
            "export function f(dev: boolean) {\n  const name = (dev && 'dev.json') || \
             'prod.json';\n  const p = join(__dirname, name);\n  readFileSync(p);\n}\n"
        ),
        "fs_read dir:dev.json | prod.json"
    );
    // The number 0 is falsy.
    assert_eq!(
        show(
            "const retries = 0;\nconst p = join(__dirname, 'fx', 'r' + (retries || \
             'none'));\nreadFileSync(p);\n"
        ),
        "fs_read dir:fx/r0 | fx/rnone"
    );
    // An empty string on the left is the result.
    assert_eq!(
        show("const sub = '';\nconst p = join(__dirname, 'fx', sub && 'nope');\nreaddirSync(p);\n"),
        "fs_read_dir dir:fx/nope | fx"
    );
}

// ---- containers ----

#[test]
fn a_call_that_returns_its_receiver_is_the_container() {
    // `s.add(a).add(b)`: both elements.
    assert_eq!(
        show(
            "const s = new Set<string>();\ns.add('s-1.json').add('s-2.json');\nfor (const f of s) \
             readFileSync(join(__dirname, f));\n"
        ),
        "fs_read dir:s-2.json | s-1.json"
    );
    // `sort()` returns the array itself: a push through the result changes it.
    assert_eq!(
        show(
            "const files = ['z.json', 'y.json'];\nconst sorted = \
             files.sort();\nsorted.push('pushed.json');\nfor (const f of files) \
             readFileSync(join(__dirname, f));\n"
        ),
        "fs_read dir:./* | z.json | y.json !partial_specifier/unknown"
    );
    // A dropped result is no alias.
    assert_eq!(
        show(
            "const s = new Set<string>();\n['a.json', 'b.json'].forEach((x) => s.add(x));\nconst \
             sorted = ['d.json', 'c.json'];\nsorted.sort();\nfor (const f of s) \
             readFileSync(join(__dirname, f));\nfor (const f of sorted) \
             readFileSync(join(__dirname, f));\n"
        ),
        "fs_read dir:a.json | b.json\nfs_read dir:d.json | c.json"
    );
}

#[test]
fn methods_of_an_object_literal_named_like_builtins() {
    // `set` and `add` of the literal write another property through `this`.
    assert_eq!(
        show(
            "const reg = { dir: 'd1', set(k: string, v: string) { this.dir = v; } \
             };\nreg.set('x', 'd2');\nreadFileSync(join(__dirname, reg.dir, 'f.json'));\nconst \
             reg2 = { dir: 'e1', add(v: string) { this.dir = v; } \
             };\nreg2.add('e2');\nreadFileSync(join(__dirname, reg2.dir, 'f.json'));\n"
        ),
        "fs_read dir:d1/f.json | ./*/f.json !partial_specifier/unknown\nfs_read dir:e1/f.json | \
         ./*/f.json !partial_specifier/unknown"
    );
    // A setter that uses `this`, an inherited property.
    assert_eq!(
        show(
            "const c = { k1: 'a.json', set sk(v: string) { this.k1 = v; } };\nc.sk = \
             'b.json';\nreadFileSync(join(__dirname, 'd', c.k1));\nconst base = { inherited: \
             'p.json' };\nconst child = { __proto__: base };\nreadFileSync(join(__dirname, 'e', \
             child.inherited));\n"
        ),
        "fs_read dir:d/a.json | d/* !partial_specifier/unknown\nfs_read dir:e/* \
         !partial_specifier/unknown"
    );
}

#[test]
fn a_local_value_named_like_a_builtin_is_not_one() {
    // `util.fill(t)` of the file's own `util`, a promise's `resolve`, a tag
    // function.
    assert_eq!(
        show(
            "const util = { fill(t: string[]) { t.push('filled.json'); } };\nconst t2 = \
             ['t2.json'];\nutil.fill(t2);\nfor (const f of t2) readFileSync(join(__dirname, 'a', \
             f));\nconst queue = ['q-1.json'];\nnew Promise((resolve) => resolve(queue)).then((q: \
             any) => q.push('q-2.json'));\nfor (const f of queue) readFileSync(join(__dirname, \
             'b', f));\nconst tagged = ['g-1.json'];\nconst tag = (strs: any, arr: string[]) => { \
             arr.push('g-2.json'); };\ntag`${tagged}`;\nfor (const f of tagged) \
             readFileSync(join(__dirname, 'c', f));\n"
        ),
        "fs_read dir:a/t2.json | a/* !partial_specifier/unknown\nfs_read dir:b/q-1.json | b/* \
         !partial_specifier/unknown\nfs_read dir:c/g-1.json | c/* !partial_specifier/unknown"
    );
    // The real ones are still pure.
    assert_eq!(
        show(
            "const t = ['a.json'];\nconsole.log(t);\nexpect(t).toHaveLength(1);\nJSON.\
             stringify(t);\njoin(...t);\nfor (const f of t) readFileSync(join(__dirname, f));\n"
        ),
        "fs_read dir:a.json"
    );
}

#[test]
fn copies_with_new_elements_and_shallow_freezes() {
    assert_eq!(
        show(
            "const xs = ['w-1.json'];\nfor (const f of xs.with(0, 'w-2.json')) \
             readFileSync(join(__dirname, f));\nfor (const f of xs.toSpliced(0, 0, 'w-3.json')) \
             readFileSync(join(__dirname, f));\n"
        ),
        "fs_read dir:w-1.json | w-2.json\nfs_read dir:w-1.json | w-3.json"
    );
    // `Object.freeze` does not freeze a nested array: importers can push to it.
    assert_eq!(
        show(
            "export const cfg = Object.freeze({ list: ['a.json'] });\nfor (const f of cfg.list) \
             readFileSync(join(__dirname, 'x', f));\nexport const flat = \
             Object.freeze(['b.json']);\nfor (const f of flat) readFileSync(join(__dirname, 'y', \
             f));\n"
        ),
        "fs_read dir:x/a.json | x/* !partial_specifier/unknown\nfs_read dir:y/b.json"
    );
}

// ---- class fields ----

#[test]
fn a_private_field_written_through_another_reference() {
    // Another instance, written inside the class.
    assert_eq!(
        show(
            "class A {\n  private dir = 'a-dir';\n  static make() { const a = new A(); a.dir = \
             'b-dir'; return a; }\n  read() { return readFileSync(join(__dirname, 'fx', this.dir, \
             'f.json')); }\n}\n"
        ),
        "fs_read dir:fx/*/f.json !partial_specifier/unknown"
    );
    // `#x` of another instance, an alias of `this`.
    assert_eq!(
        show(
            "class P {\n  #file = 'pr-1.json';\n  \
             static patch(p: P) { p.#file = 'pr-2.json'; }\n  \
             read() { return readFileSync(join(__dirname, 'a', this.#file)); }\n}\n\
             class Q {\n  private file = 'al-1.json';\n  \
             swap() { const self = this; self.file = 'al-2.json'; }\n  \
             read() { return readFileSync(join(__dirname, 'b', this.file)); }\n}\n"
        ),
        "fs_read dir:a/* !partial_specifier/unknown\nfs_read dir:b/* !partial_specifier/unknown"
    );
    // `this.n++`.
    assert_eq!(
        show(
            "class Counter {\n  private n = 0;\n  next() { this.n++; return \
             readFileSync(join(__dirname, 'c' + this.n + '.json')); }\n}\n"
        ),
        "fs_read dir:c*.json !partial_specifier/unknown"
    );
    // Only written through `this`: still the join of the writes.
    assert_eq!(
        show(
            "class K {\n  private dir = 'one';\n  set() { this.dir = 'two'; }\n  read() { return \
             readFileSync(join(__dirname, this.dir, 'f.json')); }\n}\n"
        ),
        "fs_read dir:one/f.json | two/f.json"
    );
}

#[test]
fn a_readonly_field_can_be_redeclared_by_a_subclass() {
    assert_eq!(
        show(
            "export class Base {\n  protected readonly name: string = 'base';\n  load() { return \
             readFileSync(join(__dirname, 'fx', this.name + '.json')); }\n}\nclass Derived \
             extends Base {\n  protected readonly name: string = 'derived';\n}\n"
        ),
        "fs_read dir:fx/*.json !partial_specifier/unknown"
    );
    // `private readonly` cannot be redeclared.
    assert_eq!(
        show(
            "export class Base {\n  private readonly name: string = 'base';\n  load() { return \
             readFileSync(join(__dirname, 'fx', this.name + '.json')); }\n}\n"
        ),
        "fs_read dir:fx/base.json"
    );
}

// ---- bindings ----

#[test]
fn redeclarations_recursion_and_block_functions() {
    // More redeclarations than the engine joins: one of them is unknown.
    assert_eq!(
        show_as(
            "var v = 'v0';\nvar v = 'v1';\nvar v = 'v2';\nvar v = 'v3';\nvar v = 'v4';\nvar v = \
             'v5';\nvar v = 'v6';\nvar v = 'v7';\nvar v = 'v8';\nvar v = \
             'v9';\nreadFileSync(join(__dirname, 'fx', v));\n",
            Dialect::JS,
            "main.js"
        ),
        "fs_read dir:fx/* | fx/v0 | fx/v1 | fx/v2 | fx/v3 | fx/v4 | fx/v5 | fx/v6 | fx/v7 \
         !partial_specifier/unknown"
    );
    // A named function expression that is called at once and calls itself.
    assert_eq!(
        show(
            "(function rec(p: string) {\n  readFileSync(join(__dirname, 'fx', p));\n  if (p === \
             'first.json') rec('second.json');\n})('first.json');\n"
        ),
        "fs_read dir:fx/first.json | fx/* !partial_specifier/unknown"
    );
    // Sloppy code (a script without `import`): functions of one name in two blocks.
    let src = "var fs = require('fs');\nvar path = require('path');\nif (process.env.X) { \
               function pick() { return 'a.json'; } }\nelse { function pick() { return 'b.json'; \
               } }\nfs.readFileSync(path.join(__dirname, 'fx', pick()));\n";
    let ex = crate::extract::scan_named(
        src.as_bytes(),
        Dialect::JS,
        crate::extract::ScanOptions::default(),
        Some("main.cjs"),
    );
    let read: Vec<String> = ex
        .refs
        .iter()
        .filter(|r| r.kind == turborepo_trace_core::RefKind::FsRead)
        .map(|r| r.spec.to_string())
        .collect();
    assert_eq!(read, ["fx/a.json | fx/*"]);
}

// ---- strings, promises, enums ----

#[test]
fn replacement_patterns_and_trimmed_characters() {
    // `$&` in a replacement is the match, `$$` a dollar.
    assert_eq!(
        show(
            "const name = 'x.ts';\nreadFileSync(join(__dirname, 'a', name.replace('.ts', \
             '$&x')));\nreadFileSync(join(__dirname, 'b', name.replace('.ts', '.js')));\n"
        ),
        "fs_read dir:a/* !partial_specifier/unknown\nfs_read dir:b/x.js"
    );
    // `trim` removes a byte-order mark.
    assert_eq!(
        show("readFileSync(join(__dirname, '\\u{FEFF}bom.json '.trim()));\n"),
        "fs_read dir:bom.json"
    );
}

#[test]
fn the_second_callback_of_then_and_enum_reverse_mappings() {
    assert_eq!(
        show(
            "export async function f(p: Promise<void>) {\n  const name = await p.then(() => \
             'ok.json', () => 'rejected.json');\n  return readFileSync(join(__dirname, \
             name));\n}\n"
        ),
        "fs_read dir:ok.json | rejected.json"
    );
    assert_eq!(
        show(
            "enum Num { Zero, One, Two }\nreadFileSync(join(__dirname, `name-${Num[1]}.json`));\n"
        ),
        "fs_read dir:name-One.json"
    );
}

// ---- inlined calls ----

#[test]
fn arguments_alias_the_parameters_of_an_inlined_call() {
    // Sloppy code: `arguments[0] = arguments[1]` changes `p`.
    let src = "var fs = require('fs');\nvar path = require('path');\nfunction pick(p, q) { \
               arguments[0] = arguments[1]; return p; }\nfs.readFileSync(path.join(__dirname, \
               'fx', pick('a.json', 'b.json')));\n";
    let ex = crate::extract::scan_named(
        src.as_bytes(),
        Dialect::JS,
        crate::extract::ScanOptions::default(),
        Some("main.cjs"),
    );
    let read: Vec<String> = ex
        .refs
        .iter()
        .filter(|r| r.kind == turborepo_trace_core::RefKind::FsRead)
        .map(|r| r.spec.to_string())
        .collect();
    assert_eq!(read, ["fx/a.json | fx/*"]);
}

#[test]
fn an_iteration_callback_gets_the_container_itself() {
    assert_eq!(
        show(
            "const c = ['a.json', 'b.json'];\nc.forEach((x, i, all) => { if (i === 0) \
             all.push('c.json'); });\nfor (const f of c) readFileSync(join(__dirname, 'fx', f));\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json | fx/* !partial_specifier/unknown"
    );
    // Reading through it is fine.
    assert_eq!(
        show(
            "const c = ['a.json', 'b.json'];\nc.forEach((x, i, all) => { console.log(all.length, \
             i); });\nfor (const f of c) readFileSync(join(__dirname, 'fx', f));\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json"
    );
}

#[test]
fn reordered_arrays_have_no_positions() {
    // `reverse()` in place: index 0 is the old last element.
    assert_eq!(
        show(
            "const c = ['a.json', 'b.json'];\nc.reverse();\nreadFileSync(join(__dirname, 'fx', \
             c[0]));\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json"
    );
    // Through a parameter.
    assert_eq!(
        show(
            "const c = ['a.json', 'b.json'];\nfunction rev(t: string[]) { t.sort(); \
             }\nrev(c);\nreadFileSync(join(__dirname, 'fx', c[0]));\n"
        ),
        "fs_read dir:fx/a.json | fx/* !partial_specifier/unknown"
    );
    // A sorted copy changes nothing.
    assert_eq!(
        show(
            "const c = ['b.json', 'a.json'];\nconst sorted = \
             [...c].sort();\nreadFileSync(join(__dirname, 'fx', c[0]));\n"
        ),
        "fs_read dir:fx/b.json"
    );
}

#[test]
fn a_key_of_an_open_map_may_be_missing() {
    // `m.set(k, v)` with an unknown key: `m.get('k3')` is `v` or undefined.
    assert_eq!(
        show(
            "const m = new Map([['k1', 'a.json'], ['k2', 'b.json']]);\nexport function put(k: \
             string) { m.set(k, 'c.json'); }\nreadFileSync(join(__dirname, 'fx', m.get('k3') || \
             m.get('k2')));\n"
        ),
        "fs_read dir:fx/c.json | fx/b.json"
    );
}

// ---- temporary directory, URLs, patterns ----

#[test]
fn a_module_loaded_from_the_temporary_directory_keeps_a_record() {
    // The bundle is generated at runtime from files this site cannot name; the data
    // file next to it is the program's own.
    assert_eq!(
        show(
            "export async function analyze(build: (out: string) => void) {\n  const dir = await \
             mkdtemp(join(tmpdir(), 'x-'));\n  const out = join(dir, 'out.js');\n  build(out);\n  \
             const text = readFileSync(join(dir, 'data.json'), 'utf8');\n  return [require(out), \
             text];\n}\n"
        ),
        "require dir:<unknown> !computed_specifier/data"
    );
}

#[test]
fn url_queries_fragments_and_escapes_are_not_part_of_the_path() {
    assert_eq!(
        show(
            "readFileSync(new URL('./fx/a.json?raw', import.meta.url));\nreadFileSync(new \
             URL('./fx/sp%61ce.json#top', import.meta.url));\n"
        ),
        "fs_read dir:fx/a.json\nfs_read dir:fx/space.json | fx/sp%61ce.json"
    );
}

#[test]
fn dotdot_after_an_unknown_segment() {
    // `name` can be one segment (`fx/sib.json`) or several (`fx/a/sib.json`).
    assert_eq!(
        show(
            "export function f(name: string) {\n  return readFileSync(join(__dirname, 'fx', name, \
             '..', 'sib.json'));\n}\n"
        ),
        "fs_read dir:fx/*sib.json !partial_specifier/unknown"
    );
}

// ---- values that were "never a path" (the Void audit) ----

#[test]
fn rows_of_an_imported_table_are_not_undefined() {
    // The second parameter of an `each` callback over a table from another module.
    assert_eq!(
        show(
            "import { cases } from './cases';\nit.each(cases)('%s', (name, file) => {\n  \
             readFileSync(join(__dirname, 'fx', file));\n});\n"
        ),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
    // A row of a literal table that is not an array is the only argument.
    assert_eq!(
        show(
            "it.each(['a.json', 'b.json'])('%s', (file, missing) => {\n  \
             readFileSync(join(__dirname, 'fx', missing ?? file));\n});\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json"
    );
}

#[test]
fn an_undefined_extension_and_a_colon_in_a_file_name() {
    // `path.basename(p, undefined)` is `path.basename(p)`.
    assert_eq!(
        show(
            "let ext;\nconst p = join(__dirname, 'fx', path.basename('/x/a.json', \
             ext));\nreadFileSync(p);\n"
        ),
        "fs_read dir:fx/a.json"
    );
    // A `:` after the first segment is no URL scheme.
    assert_eq!(
        show("readFileSync(new URL('./logs/10:30.txt', import.meta.url));\n"),
        "fs_read dir:logs/10:30.txt"
    );
    assert_eq!(
        show("readFileSync(new URL('https://example.com/x', import.meta.url));\n"),
        ""
    );
}

// ---- nested containers ----

#[test]
fn a_nested_container_handed_to_code_outside_the_file() {
    // `fill` is imported: it can push to the array it is given.
    assert_eq!(
        show(
            "import { fill } from './fill';\nconst cfg = { list: ['a.json'], name: 'x' \
             };\nfill(cfg.list);\nfor (const f of cfg.list) readFileSync(join(__dirname, 'fx', \
             f));\n"
        ),
        "fs_read dir:fx/a.json | fx/* !partial_specifier/unknown"
    );
    // Returned from an exported function.
    assert_eq!(
        show(
            "const cfg = { list: ['a.json'] };\nexport function files() { return cfg.list; }\nfor \
             (const f of cfg.list) readFileSync(join(__dirname, 'fx', f));\n"
        ),
        "fs_read dir:fx/a.json | fx/* !partial_specifier/unknown"
    );
    // The container is assigned to the property after the literal.
    assert_eq!(
        show(
            "import { fill } from './fill';\nconst cfg: Record<string, string[]> = {};\ncfg.list \
             = ['a.json'];\nfill(cfg.list);\nfor (const f of cfg.list) \
             readFileSync(join(__dirname, 'fx', f));\n"
        ),
        "fs_read dir:fx/a.json | fx/* !partial_specifier/unknown"
    );
    // A string property handed out changes nothing; nor does a pure reader.
    assert_eq!(
        show(
            "import { log } from './log';\nconst cfg = { list: ['a.json'], name: 'x' \
             };\nlog(cfg.name);\nconsole.log(cfg.list);\nexpect(cfg.list).toHaveLength(1);\nfor \
             (const f of cfg.list) readFileSync(join(__dirname, 'fx', f));\n"
        ),
        "fs_read dir:fx/a.json"
    );
}

// ---- the token evaluator alone ----

#[test]
fn a_site_the_engine_does_not_answer_keeps_a_record() {
    // `eval` makes the engine decline the file. The token evaluator reads the
    // top-level `dir` although the parameter shadows it: the reference stays,
    // with an open record.
    assert_eq!(
        show_as(
            "eval(code);\nconst dir = 'top';\nfunction f(dir) { return \
             readFileSync(join(__dirname, 'fx', dir)); }\n",
            Dialect::JS,
            "main.js"
        ),
        "fs_read dir:fx/top !computed_specifier/unknown"
    );
    // A top-level `let` read by its initial value.
    assert_eq!(
        show_as(
            "eval(code);\nlet mode = 'm1';\nmode = 'm2';\nreadFileSync(join(__dirname, 'fx', \
             mode));\n",
            Dialect::JS,
            "main.js"
        ),
        "fs_read dir:fx/m1 !computed_specifier/unknown"
    );
    // One string literal needs no check.
    assert_eq!(
        show_as("eval(code);\nrequire('./x');\n", Dialect::JS, "main.js"),
        "require dir:./x"
    );
    // With the engine, the parameter is what the site reads.
    assert_eq!(
        show(
            "const dir = 'top';\nexport function f(dir: string) { return \
             readFileSync(join(__dirname, 'fx', dir)); }\n"
        ),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
}

#[test]
fn a_complete_token_result_is_checked_too() {
    // A parameter named like the module: the token evaluator sees `path.join`.
    assert_eq!(
        show(
            "function f(path: { join(...p: string[]): string }) {\n  return \
             readFileSync(path.join(__dirname, 'fx', 'n.json'));\n}\nf({ join: (...p: string[]) \
             => p[0] + '/elsewhere/' + p[2] });\n"
        ),
        "fs_read dir:elsewhere/n.json"
    );
    // `flag && '-on'` reads no constant, and is still not always `-on`.
    assert_eq!(
        show(
            "export function f(flag: boolean) {\n  return readFileSync(join(__dirname, 'fx', 'x' \
             + (flag && '-on') + '.json'));\n}\n"
        ),
        "fs_read dir:fx/x-on.json | fx/x*.json !partial_specifier/unknown"
    );
    // `new URL(a, base).href` as an argument: the site is the `new` expression.
    assert_eq!(
        show(
            "const root = new URL('../', import.meta.url);\nexport const load = (next: (u: \
             string) => unknown) =>\n  next(new URL('agent/lib/wire.ts', root).href);\n"
        ),
        "fs_read dir:..\nfs_read dir:../agent/lib/wire.ts"
    );
}

#[test]
fn chains_nested_in_brackets_are_not_parsed() {
    // 40 bracket levels of 100 curried arrows each: every level is short, the tree
    // is 4,000 functions deep. The engine declines (and the stack survives);
    // the site keeps the token result with a record.
    let mut deep = String::from("1");
    for _ in 0..40 {
        deep = format!("{}({deep})", "a => ".repeat(100));
    }
    let body = format!(
        "const deep = {deep};\nexport function f(dir: string) {{ return \
         readFileSync(join(__dirname, dir)); }}\n"
    );
    assert!(!super::shallow_enough(
        format!("{}{body}", "").as_bytes(),
        Dialect::TS
    ));
    assert_eq!(show(&body), "fs_read dir:./* !computed_specifier/unknown");
    // One long chain alone is fine.
    let flat = format!("const f = {}1;\n", "a => ".repeat(1000));
    assert!(super::shallow_enough(flat.as_bytes(), Dialect::TS));
}

// ---- bounds ----

#[test]
fn a_container_whose_uses_are_not_checked_is_open() {
    // Five containers, each filled from the next: the innermost is past the nesting
    // bound.
    assert_eq!(
        show(
            "const a: string[] = [];\nconst b: string[] = [];\nconst c: string[] = [];\nconst d: \
             string[] = [];\nconst e: string[] = [];\nconst f: string[] = \
             [];\na.push(...b);\nb.push(...c);\nc.push(...d);\nd.push(...e);\ne.push(...f);\nf.\
             push('deep.json');\nfor (const x of a) readFileSync(join(__dirname, 'fx', x));\nfor \
             (const x of f) readFileSync(join(__dirname, 'gx', x));\n"
        ),
        "fs_read dir:fx/* !partial_specifier/unknown\nfs_read dir:gx/deep.json"
    );
    // A push computed from the container itself.
    assert_eq!(
        show(
            "const a = ['a'];\na.push('b');\na.push(a[1] + '2');\nfor (const x of a) \
             readFileSync(join(__dirname, 'fx', x));\n"
        ),
        "fs_read dir:fx/a | fx/b | fx/* !partial_specifier/unknown"
    );
}

#[test]
fn bounds_degrade_to_unknown() {
    // More alternatives than a value keeps.
    let names: Vec<String> = (0..70).map(|i| format!("'m{i}.json'")).collect();
    assert_eq!(
        show(&format!(
            "const all = [{}];\nexport function f(i: number) {{\n  return \
             readFileSync(join(__dirname, 'fx', all[i]));\n}}\n",
            names.join(", ")
        )),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
    // More call sites than are joined for a parameter.
    let calls: String = (0..70).map(|i| format!("load('c{i}.json');\n")).collect();
    assert_eq!(
        show(&format!(
            "function load(name: string) {{ return readFileSync(join(__dirname, 'fx', name)); \
             }}\n{calls}"
        )),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
    // Inlining deeper than the bound.
    let mut chain = String::from("function f0(p: string) { return p; }\n");
    for i in 1..12 {
        chain.push_str(&format!(
            "function f{i}(p: string) {{ return f{}(p); }}\n",
            i - 1
        ));
    }
    assert_eq!(
        show(&format!(
            "{chain}readFileSync(join(__dirname, 'fx', f11('deep.json')));\n"
        )),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
}
