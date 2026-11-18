//! Tests of the value-flow engine through the extractor: one per construct,
//! plus the cases where a site must stay uncertain.

use turborepo_trace_core::{Base, RefKind};

use crate::{
    extract::{ScanOptions, scan_named},
    lexer::Dialect,
};

const HEAD: &str = "import fs, { readFileSync, readdirSync, existsSync } from 'node:fs';\nimport \
                    { readFile, readdir, mkdtemp } from 'node:fs/promises';\nimport path, { join, \
                    resolve, dirname } from 'node:path';\nimport { fileURLToPath, pathToFileURL } \
                    from 'node:url';\nimport os, { tmpdir } from 'node:os';\n";

/// The dynamic references of `HEAD + body`, one per line: `<kind>
/// <base>:<spec>` plus ` !<uncertainty kind>/<origin>`.
pub(super) fn show_as(body: &str, dialect: Dialect, name: &str) -> String {
    let src = format!("{HEAD}{body}");
    let ex = scan_named(src.as_bytes(), dialect, ScanOptions::default(), Some(name));
    let mut out = Vec::new();
    for r in &ex.refs {
        if matches!(r.kind, RefKind::Import | RefKind::Asset | RefKind::ReExport) {
            continue;
        }
        let base = match r.base {
            Base::FileDir => "dir",
            Base::Cwd => "cwd",
            // Next to a resolved module.
            Base::Language(crate::extract::evaluator::RESOLVED_BASE) => "mod",
            // Module strings in registry position (another mechanism, with its own tests).
            _ => continue,
        };
        let mut line = format!("{} {base}:{}", r.kind.as_str(), r.spec);
        if let Some(u) = &r.uncertainty {
            line.push_str(&format!(" !{}/{}", u.kind.as_str(), u.origin.as_str()));
        }
        out.push(line);
    }
    out.join("\n")
}

pub(super) fn show(body: &str) -> String {
    show_as(body, Dialect::TS, "main.ts")
}

/// The detail of the first uncertain reference.
fn detail(body: &str) -> String {
    let src = format!("{HEAD}{body}");
    let ex = scan_named(
        src.as_bytes(),
        Dialect::TS,
        ScanOptions::default(),
        Some("main.ts"),
    );
    ex.refs
        .iter()
        .filter(|r| !matches!(r.kind, RefKind::Asset))
        .find_map(|r| r.uncertainty.as_ref().map(|u| u.detail.to_string()))
        .unwrap_or_default()
}

#[test]
fn pipeline_steps() {
    let src = "import { readFileSync } from 'node:fs';\nimport { join } from \
               'node:path';\nfunction load() { const dir = join(__dirname, 'f'); return \
               readFileSync(join(dir, 'a')); }\n";
    assert!(super::shallow_enough(src.as_bytes(), Dialect::TS));
    let start = src.find("join(dir").unwrap() as u32;
    let span = turborepo_trace_core::Span::new(start, start + "join(dir, 'a')".len() as u32);
    let direct = super::analyze(
        src,
        Dialect::TS,
        Some("main.ts"),
        &[(span, super::Ask::Value)],
        false,
        None,
    );
    assert!(direct.is_some(), "analyze failed");
    assert!(direct.unwrap().sites[0].is_some(), "site not found");
    let threaded = super::evaluate_sites(
        src.as_bytes(),
        Dialect::TS,
        Some("main.ts"),
        &[(span, super::Ask::Value)],
    );
    assert!(threaded.is_some(), "thread failed");
}

// ---- bindings ----

#[test]
fn function_local_const() {
    assert_eq!(
        show(
            "function load() {\n  const dir = join(__dirname, 'fixtures');\n  return \
             readFileSync(join(dir, 'a.json'), 'utf8');\n}\n"
        ),
        "fs_read dir:fixtures/a.json"
    );
    // Nested scopes and block scoping.
    assert_eq!(
        show(
            "export function a() {\n  const dir = join(__dirname, 'a');\n  if (x) { const dir = \
             join(__dirname, 'b'); readFileSync(join(dir, 'f')); }\n  readFileSync(join(dir, \
             'f'));\n}\n"
        ),
        "fs_read dir:b/f\nfs_read dir:a/f"
    );
}

#[test]
fn esm_dirname() {
    assert_eq!(
        show(
            "const __filename = fileURLToPath(import.meta.url);\nconst __dirname = \
             dirname(__filename);\nexport function f() { const p = join(__dirname, '..', 'data', \
             'x.json'); return readFileSync(p); }\n"
        ),
        "fs_read dir:../data/x.json"
    );
}

#[test]
fn let_without_initializer_joins_every_assignment() {
    assert_eq!(
        show(
            "let file;\nif (process.env.CI) { file = join(__dirname, 'ci.json'); } else { file = \
             join(__dirname, 'local.json'); }\nreadFileSync(file);\n"
        ),
        "fs_read dir:ci.json | local.json"
    );
    // `var` hoisting and assignment in a closure.
    assert_eq!(
        show(
            "function init() { dir = join(__dirname, 'late'); }\nvar dir = join(__dirname, \
             'early');\nfunction read() { return readFileSync(join(dir, 'x')); }\n"
        ),
        "fs_read dir:early/x | late/x"
    );
}

#[test]
fn assignment_of_an_unknown_value_keeps_the_site_uncertain() {
    assert_eq!(
        show("let p = join(__dirname, 'a.json');\np = compute();\nreadFileSync(p);\n"),
        "fs_read dir:a.json\nfs_read dir:<unknown> !computed_specifier/call"
    );
}

#[test]
fn logical_assignment_and_defaults() {
    assert_eq!(
        show(
            "let dir: string | undefined;\ndir ??= join(__dirname, 'd');\nreadFileSync(join(dir, \
             'a'));\n"
        ),
        "fs_read dir:d/a"
    );
    assert_eq!(
        show("function f(dir = join(__dirname, 'd')) { return readdirSync(dir); }\nf();\n"),
        "fs_read_dir dir:d"
    );
}

#[test]
fn self_referential_assignment() {
    assert_eq!(
        show(
            "let dir = join(__dirname, 'a');\ndir = path.resolve(dir, \
             'b');\nreadFileSync(join(dir, 'x'));\n"
        ),
        // Flow-insensitive: before and after the reassignment (and again).
        "fs_read dir:a/x | a/b/x | a/b/b/x\nfs_read dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn destructuring() {
    assert_eq!(
        show(
            "const paths = { fixtures: join(__dirname, 'fixtures'), names: ['a.json', 'b.json'] \
             };\nconst { fixtures, names: [first, second = 'z.json'] } = \
             paths;\nreadFileSync(join(fixtures, first));\nreadFileSync(join(fixtures, second));\n"
        ),
        "fs_read dir:fixtures/a.json\nfs_read dir:fixtures/b.json"
    );
    // A default applies when the property may be missing.
    assert_eq!(
        show(
            "export function f(opts: { dir?: string }) {\n  const { dir = join(__dirname, 'd') } \
             = opts;\n  return readdirSync(dir);\n}\n"
        ),
        "fs_read_dir dir:d\nfs_read_dir dir:<unknown> !computed_specifier/param"
    );
}

// ---- objects and arrays ----

#[test]
fn object_lookup_with_unknown_key_reads_every_value() {
    assert_eq!(
        show(
            "const ROUTES = { a: './routes/a', b: './routes/b' } as const;\nexport function \
             load(name: keyof typeof ROUTES) { return require(ROUTES[name]); }\n"
        ),
        "require dir:./routes/a | ./routes/b"
    );
    assert_eq!(
        show(
            "const DIRS = { a: join(__dirname, 'a'), b: join(__dirname, 'b') };\nexport function \
             f(k: 'a' | 'b') { return readdirSync(DIRS.a) + readdirSync(DIRS[k]); }\n"
        ),
        "fs_read_dir dir:a\nfs_read_dir dir:a | b"
    );
}

#[test]
fn loops_over_array_literals() {
    assert_eq!(
        show(
            "for (const f of ['a.json', 'b.json']) { readFileSync(join(__dirname, 'data', f)); }\n"
        ),
        "fs_read dir:data/a.json | data/b.json"
    );
    assert_eq!(
        show(
            "const FILES = ['a.json', 'b.json'];\nFILES.forEach((f) => { \
             readFileSync(join(__dirname, f)); });\nconst all = FILES.map((f) => join(__dirname, \
             'x', f));\nfor (const p of all) readFileSync(p);\nfor (const i in FILES) \
             readFileSync(join(__dirname, 'y', FILES[i]));\n"
        ),
        "fs_read dir:a.json | b.json\nfs_read dir:x/a.json | x/b.json\nfs_read dir:y/a.json | \
         y/b.json"
    );
}

#[test]
fn collections_built_in_place() {
    assert_eq!(
        show(
            "const set = new Set<string>();\nset.add(join(__dirname, 'a.json'));\nif (c) \
             set.add(join(__dirname, 'b.json'));\nfor (const p of set) readFileSync(p);\n"
        ),
        "fs_read dir:a.json | b.json"
    );
    assert_eq!(
        show(
            "const list: string[] = [];\nlist.push(join(__dirname, 'a'), join(__dirname, \
             'b'));\nconst more = [...list, join(__dirname, \
             'c')].filter(Boolean).concat([join(__dirname, 'd')]);\nfor (const p of more) \
             readdirSync(p);\n"
        ),
        "fs_read_dir dir:a | b | c | d"
    );
    assert_eq!(
        show(
            "const m = new Map<string, string>();\nm.set('a', join(__dirname, 'a.json'));\nexport \
             function f(k: string) { return readFileSync(m.get(k)!); }\n"
        ),
        "fs_read dir:a.json"
    );
}

#[test]
fn object_helpers() {
    assert_eq!(
        show(
            "const T = { a: 'a.sql', b: 'b.sql' };\nfor (const [name, file] of Object.entries(T)) \
             readFileSync(join(__dirname, name, file));\nfor (const file of Object.values(T)) \
             readFileSync(join(__dirname, file));\nfor (const name of Object.keys(T)) \
             readFileSync(join(__dirname, name));\n"
        ),
        // Keys and values of different entries are not correlated.
        "fs_read dir:a/a.sql | b/a.sql | a/b.sql | b/b.sql\nfs_read dir:a.sql | b.sql\nfs_read \
         dir:a | b"
    );
}

#[test]
fn spread_and_array_join() {
    assert_eq!(
        show(
            "const parts = ['data', 'x.json'];\nreadFileSync(join(__dirname, \
             ...parts));\nreadFileSync([__dirname, 'a', 'b.json'].join('/'));\nconst base = { \
             dir: join(__dirname, 'b') };\nconst o = { ...base, name: 'n' \
             };\nreadFileSync(join(o.dir, o.name));\n"
        ),
        "fs_read dir:data/x.json\nfs_read dir:a/b.json\nfs_read dir:b/n"
    );
}

// ---- strings ----

#[test]
fn templates_concat_and_string_methods() {
    assert_eq!(
        show(
            "export function f(name: string) {\n  const ext = '.json';\n  const p = \
             `${__dirname}/data/${name}${ext}`;\n  return readFileSync(p);\n}\n"
        ),
        "fs_read dir:data/*.json !partial_specifier/unknown"
    );
    assert_eq!(
        show(
            "const NAME = 'Config.JSON'.toLowerCase().replace('.json', \
             '.yaml');\nreadFileSync(join(__dirname, \
             String(NAME)));\nreadFileSync(path.join(__dirname, 'a').toString());\n"
        ),
        "fs_read dir:config.yaml\nfs_read dir:a"
    );
    // An unknown transformation of a known path is unknown.
    assert_eq!(
        show("const p = join(__dirname, 'a.ts').replace(/\\.ts$/, '.js');\nreadFileSync(p);\n"),
        "fs_read dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn plus_assign() {
    assert_eq!(
        show("let p = __dirname + '/data';\np += '/a.json';\nreadFileSync(p);\n"),
        "fs_read dir:data | data* !partial_specifier/unknown"
    );
}

#[test]
fn urls() {
    assert_eq!(
        show(
            "const MIGRATIONS = new URL('../migrations/', import.meta.url);\nexport function \
             read(name: string) {\n  return readFileSync(fileURLToPath(new URL(name, \
             MIGRATIONS)));\n}\nconst one = new URL('./one.sql', \
             MIGRATIONS);\nreadFileSync(one);\nreadFileSync(new URL('two.sql', \
             MIGRATIONS).pathname);\n"
        ),
        // `new URL(<literal>, <file URL>)` is itself a read (nft's asset rule): the directory,
        // and `one.sql` twice.
        "fs_read dir:../migrations\nfs_read dir:../migrations/* \
         !partial_specifier/unknown\nfs_read dir:../migrations/one.sql\nfs_read \
         dir:../migrations/one.sql\nfs_read dir:../migrations/two.sql"
    );
    // A base URL without a trailing slash names a file: its directory is the base.
    assert_eq!(
        show(
            "const here = new URL('./data/index.json', import.meta.url);\nreadFileSync(new \
             URL('./other.json', here));\n"
        ),
        "fs_read dir:data/index.json\nfs_read dir:data/other.json"
    );
}

// ---- functions ----

#[test]
fn local_function_results() {
    assert_eq!(
        show(
            "function fixturesDir() { return join(__dirname, '__fixtures__'); }\nconst fixture = \
             (name: string) => join(fixturesDir(), name);\nconst dir = \
             fixturesDir();\nreadFileSync(join(dir, \
             'a.json'));\nreadFileSync(fixture('b.json'));\n"
        ),
        "fs_read dir:__fixtures__/a.json\nfs_read dir:__fixtures__/b.json"
    );
    // Several returns, async, object methods.
    assert_eq!(
        show(
            "async function pick(kind: string) {\n  if (kind === 'a') return join(__dirname, \
             'a');\n  return join(__dirname, 'b');\n}\nconst helpers = { dir() { return \
             join(__dirname, 'h'); } };\nexport async function f(k: string) {\n  \
             readdirSync(await pick(k));\n  readdirSync(helpers.dir());\n}\n"
        ),
        "fs_read_dir dir:a | b\nfs_read_dir dir:h"
    );
}

#[test]
fn calls_are_evaluated_per_call_site() {
    // The exported helper may be called from elsewhere, but this call is fully
    // known.
    assert_eq!(
        show(
            "export function fixture(name: string) { return join(__dirname, 'fx', name); \
             }\nreadFileSync(fixture('a.json'));\n"
        ),
        "fs_read dir:fx/a.json"
    );
}

#[test]
fn recursion_is_bounded() {
    assert_eq!(
        show(
            "function up(dir: string, n: number): string { return n ? up(dirname(dir), n - 1) : \
             dir; }\nreaddirSync(up(__dirname, 2));\n"
        ),
        "fs_read_dir dir:.\nfs_read_dir dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn parameters_of_functions_only_called_in_the_file() {
    assert_eq!(
        show(
            "function parse(name: string) { return readFileSync(join(__dirname, 'fx', name)); \
             }\nparse('a.json');\nparse('b.json');\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json"
    );
    assert_eq!(
        show(
            "const load = async (opts: { dir: string; name: string }) => readFile(join(opts.dir, \
             opts.name));\nawait load({ dir: join(__dirname, 'd'), name: 'a.json' });\n"
        ),
        "fs_read dir:d/a.json"
    );
    // Destructured parameters and parameters flowing through two functions.
    assert_eq!(
        show(
            "function inner({ file }: { file: string }) { return readFileSync(file); }\nfunction \
             outer(name: string) { return inner({ file: join(__dirname, name) }); \
             }\nouter('x.json');\n"
        ),
        "fs_read dir:x.json"
    );
}

#[test]
fn parameters_of_escaping_functions_keep_a_param_record() {
    assert_eq!(
        show(
            "export function parse(p: string) { return readFileSync(p); }\nparse(join(__dirname, \
             'a.json'));\n"
        ),
        "fs_read dir:a.json\nfs_read dir:<unknown> !computed_specifier/param"
    );
    assert_eq!(
        detail("export function parse(p: string) { return readFileSync(p); }\n"),
        "p; parameter parse#0"
    );
    // Used as a value.
    assert_eq!(
        show(
            "function parse(p: string) { return readFileSync(p); \
             }\nregister(parse);\nparse(join(__dirname, 'a.json'));\n"
        ),
        "fs_read dir:a.json\nfs_read dir:<unknown> !computed_specifier/param"
    );
    // A parameter below a known directory is a pattern, not an uncertainty.
    assert_eq!(
        show(
            "export function parse(name: string) { return readFileSync(join(__dirname, 'fx', \
             name)); }\n"
        ),
        "fs_read dir:fx/* !partial_specifier/unknown"
    );
    // Closures returned from a function, methods, callbacks of unknown functions.
    assert_eq!(
        show(
            "export function reader(dir: string) { return (name: string) => \
             readFileSync(join(dir, name)); }\nconst cached = wrap(cache, (p: string) => \
             readdir(p));\nexport class P { read(file: string) { return readFile(file); } }\n"
        ),
        "fs_read dir:<unknown> !computed_specifier/param\nfs_read_dir dir:<unknown> \
         !computed_specifier/param\nfs_read dir:<unknown> !computed_specifier/param"
    );
}

#[test]
fn shadowing_a_top_level_constant() {
    // The token evaluator would take the top-level `dir`.
    assert_eq!(
        show(
            "const dir = join(__dirname, 'a');\nexport function f(dir: string) { return \
             readFileSync(join(dir, 'x.json')); }\n"
        ),
        "fs_read dir:<unknown> !computed_specifier/param"
    );
}

#[test]
fn test_tables() {
    assert_eq!(
        show(
            "const FIXTURES = ['a.json', 'b.json'];\nfunction parseFixture(name: string) { return \
             readFileSync(join(__dirname, 'fx', name)); }\ndescribe.each(FIXTURES)('parse(%s)', \
             (fixtureName) => {\n  it('parses', () => { parseFixture(fixtureName); \
             });\n});\nit.each([['x', 'x.out'], ['y', 'y.out']])('%s', (input, output) => {\n  \
             readFileSync(join(__dirname, input));\n  readFileSync(join(__dirname, \
             output));\n});\n"
        ),
        "fs_read dir:fx/a.json | fx/b.json\nfs_read dir:x | y\nfs_read dir:x.out | y.out"
    );
}

// ---- origins ----

#[test]
fn environment() {
    assert_eq!(
        show(
            "const p = process.env.DATA_FILE;\nconst data = p ? JSON.parse(readFileSync(p, \
             'utf8')) : undefined;\n"
        ),
        "fs_read dir:<unknown> !env_dependent/env"
    );
    assert_eq!(
        detail("const p = process.env.DATA_FILE;\nreadFileSync(p);\n"),
        "p; env DATA_FILE"
    );
    // A known default stays a reference; the environment part is recorded.
    assert_eq!(
        show(
            "const dir = process.env.TMP_FOLDER || path.resolve(process.cwd(), \
             '.tmp');\nreaddirSync(dir);\nconst { CONFIG_DIR = join(__dirname, 'config') } = \
             process.env;\nreadFileSync(join(CONFIG_DIR, 'a.json'));\n"
        ),
        "fs_read_dir cwd:.tmp !cwd_dependent/cwd\nfs_read_dir dir:<unknown> \
         !env_dependent/env\nfs_read dir:config/a.json\nfs_read dir:<unknown> !env_dependent/env"
    );
    // Below a known directory the variable only selects a file: a pattern.
    assert_eq!(
        show("readFileSync(join(__dirname, 'config', process.env.NODE_ENV + '.json'));\n"),
        "fs_read dir:config/*.json !partial_specifier/unknown"
    );
}

#[test]
fn temporary_directories_are_outside_the_repository() {
    assert_eq!(
        show(
            "const d = fs.mkdtempSync(join(os.tmpdir(), 'x-'));\nfs.writeFileSync(join(d, \
             'out.json'), '{}');\nreadFileSync(join(d, 'out.json'));\nexport async function t() \
             {\n  const dir = await mkdtemp(path.join(tmpdir(), 'y-'));\n  return \
             readdir(dir);\n}\nreadFileSync(join(process.env.TMPDIR ?? os.tmpdir(), 'z'));\n"
        ),
        ""
    );
    // `mkdtemp` next to the file stays inside the repository.
    assert_eq!(
        show("const d = fs.mkdtempSync(join(__dirname, 'tmp-'));\nreadFileSync(join(d, 'a'));\n"),
        "fs_read dir:tmp-*/a !partial_specifier/unknown"
    );
}

#[test]
fn argv_imports_calls_and_data() {
    assert_eq!(
        show(
            "readFileSync(process.argv[2]);\nconst [, , file] = \
             process.argv;\nreadFileSync(file);\n"
        ),
        "fs_read dir:<unknown> !computed_specifier/argv\nfs_read dir:<unknown> \
         !computed_specifier/argv"
    );
    assert_eq!(
        show(
            "import { FIXTURES, getDir } from './consts';\nimport * as dirs from \
             './dirs';\nreadFileSync(join(FIXTURES, 'a.json'));\nreadFileSync(join(getDir('x'), \
             'a.json'));\nreadFileSync(dirs.ROOT);\n"
        ),
        "fs_read dir:<unknown> !computed_specifier/imported\nfs_read dir:<unknown> \
         !computed_specifier/call\nfs_read dir:<unknown> !computed_specifier/imported"
    );
    assert_eq!(
        detail("import { FIXTURES } from './consts';\nreadFileSync(join(FIXTURES, 'a.json'));\n"),
        "join(FIXTURES, 'a.json'); imported ./consts#FIXTURES"
    );
    assert_eq!(
        show(
            "const cfg = JSON.parse(readFileSync(join(__dirname, 'c.json'), \
             'utf8'));\nreadFileSync(cfg.file);\n"
        ),
        "fs_read dir:c.json\nfs_read dir:<unknown> !computed_specifier/data"
    );
}

#[test]
fn a_join_of_origins_is_not_ignorable() {
    // A parameter joined with runtime data: only the parameter alternative is
    // `param`.
    assert_eq!(
        show(
            "export function f(p: string) {\n  const cfg = JSON.parse(readFileSync(p, 'utf8'));\n  \
             readFileSync(c ? p : cfg.other);\n  readFileSync(join(p, cfg.name));\n}\n"
        ),
        "fs_read dir:<unknown> !computed_specifier/param\n\
         fs_read dir:<unknown> !computed_specifier/param\n\
         fs_read dir:<unknown> !computed_specifier/data\n\
         fs_read dir:<unknown> !computed_specifier/data"
    );
}

#[test]
fn directory_listings() {
    assert_eq!(
        show(
            "const dir = join(__dirname, 'migrations');\nfor (const name of readdirSync(dir)) { \
             readFileSync(join(dir, name)); }\nfor (const e of fs.readdirSync(dir, { \
             withFileTypes: true })) {\n  if (e.isFile()) readFileSync(join(e.path, e.name));\n}\n"
        ),
        "fs_read_dir dir:migrations\nfs_read dir:migrations/* \
         !partial_specifier/unknown\nfs_read_dir dir:migrations\nfs_read dir:migrations/* \
         !partial_specifier/unknown"
    );
    // A listing of a parameter's directory is the parameter's business.
    assert_eq!(
        show(
            "export async function all(dir: string) {\n  for (const f of await readdir(dir)) \
             await readFile(join(dir, f));\n}\n"
        ),
        "fs_read_dir dir:<unknown> !computed_specifier/param\nfs_read dir:<unknown> \
         !computed_specifier/param"
    );
}

// ---- classes ----

#[test]
fn class_fields() {
    assert_eq!(
        show(
            "class Store {\n  private readonly dir = join(__dirname, 'store');\n  #cache = \
             join(__dirname, 'cache');\n  read(name: string) { return readFileSync(join(this.dir, \
             name)) + readdirSync(this.#cache); }\n}\n"
        ),
        "fs_read dir:store/* !partial_specifier/unknown\nfs_read_dir dir:cache"
    );
    assert_eq!(
        show(
            "class Loader {\n  constructor(private readonly root: string) {}\n  load() { return \
             readdirSync(this.root); }\n}\nnew Loader(join(__dirname, 'r')).load();\n"
        ),
        "fs_read_dir dir:<unknown> !computed_specifier/param"
    );
    // A public field can be written by anyone.
    assert_eq!(
        show(
            "export class Open {\n  dir = join(__dirname, 'd');\n  list() { return \
             readdirSync(this.dir); }\n}\n"
        ),
        "fs_read_dir dir:<unknown> !computed_specifier/unknown"
    );
}

// ---- mutable state ----

// rustfmt's `format_strings` would change this item's string literals.
#[rustfmt::skip]
#[test]
fn containers_passed_to_unknown_code_are_not_trusted() {
    assert_eq!(
        show(
            "const cfg = { dir: join(__dirname, 'd') };\nmutate(cfg);\n\
             readFileSync(join(cfg.dir, 'a.json'));\n"
        ),
        "fs_read dir:d/a.json\nfs_read dir:<unknown> !computed_specifier/unknown"
    );
    assert_eq!(
        show(
            "export const cfg = { dir: join(__dirname, 'd') };\n\
             readFileSync(join(cfg.dir, 'a.json'));\n"
        ),
        "fs_read dir:d/a.json\nfs_read dir:<unknown> !computed_specifier/unknown"
    );
    // Frozen exports and containers that are only read are fine.
    assert_eq!(
        show(
            "export const cfg = { dir: join(__dirname, 'd') } as const;\n\
             console.log(cfg);\nexpect(Object.keys(cfg)).toEqual(['dir']);\n\
             readFileSync(join(cfg.dir, 'a.json'));\n"
        ),
        "fs_read dir:d/a.json"
    );
}

#[test]
fn property_writes_are_part_of_the_value() {
    assert_eq!(
        show(
            "const cfg: Record<string, string> = { dir: join(__dirname, 'd') };\nif (x) cfg.dir = \
             join(__dirname, 'e');\ncfg.other = join(__dirname, \
             'o');\nreaddirSync(cfg.dir);\nreaddirSync(cfg.other);\n"
        ),
        "fs_read_dir dir:d | e\nfs_read_dir dir:o"
    );
    // A write through an alias is seen by reads of that property anywhere.
    assert_eq!(
        show(
            "const cfg = { inner: { dir: join(__dirname, 'd') } };\nconst i = cfg.inner;\ni.dir = \
             other();\nreadFileSync(join(cfg.inner.dir, 'a.json'));\n"
        ),
        "fs_read dir:d/a.json\nfs_read dir:<unknown> !computed_specifier/unknown"
    );
    assert_eq!(
        show(
            "function f(opts: { dir: string }) { opts.dir = other(); return \
             readdirSync(opts.dir); }\nf({ dir: join(__dirname, 'd') });\n"
        ),
        "fs_read_dir dir:d\nfs_read_dir dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn bindings_assigned_in_other_functions() {
    assert_eq!(
        show(
            "let dir = join(__dirname, 'a');\nexport function setDir(d: string) { dir = d; \
             }\nexport function read() { return readFileSync(join(dir, 'x.json')); }\n"
        ),
        "fs_read dir:a/x.json\nfs_read dir:<unknown> !computed_specifier/param"
    );
}

#[test]
fn arguments_object_aliases_parameters() {
    assert_eq!(
        show_as(
            "function f(p) { arguments[0] = other(); return readFileSync(p); }\nf(join(__dirname, \
             'a.json'));\n",
            Dialect::JS,
            "main.js"
        ),
        "fs_read dir:a.json\nfs_read dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn eval_keeps_the_token_results() {
    assert_eq!(
        show_as(
            "eval(code);\nfunction g() { const d = join(__dirname, 'x'); return readFileSync(d); \
             }\n",
            Dialect::JS,
            "main.js"
        ),
        "fs_read dir:<unknown> !computed_specifier/unknown"
    );
}

/// Values the engine must not mistake for "never a path" (each was a site that
/// lost its record on a real repository).
#[test]
fn derived_values_are_not_void() {
    // Regex matches are strings derived from the text.
    assert_eq!(
        show(
            "const text = readFileSync(join(__dirname, 'a.txt'), 'utf8');\nfor (const m of \
             text.matchAll(/x(.)/g)) {\n  const cleaned = m[1].replace(/^\\.\\//, '');\n  \
             readFileSync(join(__dirname, 'd', cleaned));\n}\n"
        ),
        "fs_read dir:a.txt\nfs_read dir:d/* !partial_specifier/unknown"
    );
    // A URL against a base that is not a file URL is unknown, not "another scheme".
    assert_eq!(
        show(
            "const root = new URL('.', somewhere.href);\nawait import(new URL('lib/x.js', \
             root).href);\n"
        ),
        "dynamic_import dir:<unknown> !computed_specifier/unknown"
    );
    // `then` returns what its callback returns.
    assert_eq!(
        show(
            "export async function f() {\n  const first = await readdir(join(__dirname, \
             'd')).then((names) => join(__dirname, 'e', names[0]));\n  return \
             readFile(first);\n}\n"
        ),
        "fs_read_dir dir:d\nfs_read dir:e/* !partial_specifier/unknown"
    );
    // A method of an unknown object named like a string predicate can return
    // anything.
    assert_eq!(
        show("export function g(repo: any) { return readFileSync(repo.search('x')); }\n"),
        "fs_read dir:<unknown> !computed_specifier/param"
    );
    // A character of a string, a chained `Set.add`, an injected class field.
    assert_eq!(
        show(
            "const names = 'ab';\nreadFileSync(join(__dirname, 'c', names[0]));\nconst s = new \
             Set<string>().add(join(__dirname, 'x'));\nfor (const p of s) readdirSync(p);\nclass \
             S { @Inject() private readonly dir!: string; list() { return readdirSync(this.dir); \
             } }\n"
        ),
        "fs_read dir:c/* !partial_specifier/unknown\nfs_read_dir dir:x\nfs_read_dir dir:<unknown> \
         !computed_specifier/unknown"
    );
}

// ---- module loads ----

#[test]
fn module_loads() {
    assert_eq!(
        show(
            "export async function load(name: string) {\n  const mod = './handlers/' + name + \
             '.js';\n  return import(mod);\n}\nconst PLUGINS = ['./plugins/a', \
             './plugins/b'];\nfor (const p of PLUGINS) require(p);\nconst entry = \
             pathToFileURL(join(__dirname, 'dist', 'index.js')).href;\nawait import(entry);\n"
        ),
        "dynamic_import dir:./handlers/*.js !partial_specifier/unknown\nrequire dir:./plugins/a | \
         ./plugins/b\ndynamic_import dir:./dist/index.js"
    );
    assert_eq!(
        show("export const load = (spec: string) => import(spec);\n"),
        "dynamic_import dir:<unknown> !computed_specifier/param"
    );
}

#[test]
fn registries_with_many_values_stay_exact() {
    let mut body = String::from("const R = {\n");
    for i in 0..20 {
        body.push_str(&format!("  k{i}: 'services/s{i}/src/routes',\n"));
    }
    body.push_str(
        "} as const;\nexport function load(n: keyof typeof R) { return require(R[n]); }\n",
    );
    let out = show(&body);
    assert!(
        out.starts_with("require dir:services/s0/src/routes | services/s1/src/routes"),
        "{out}"
    );
    assert_eq!(out.matches(" | ").count(), 19, "{out}");
}

/// A `Set` filled from a local function's result and from lookups in a constant
/// table, then iterated for `require`: every table value and the cwd-relative
/// path.
#[test]
fn route_registry_built_into_a_set() {
    let mut body = String::from("const ROUTES = {\n");
    for i in 0..18 {
        body.push_str(&format!("  'api-{i}': 'services/svc-a-{i}/src/routes',\n"));
    }
    body.push_str(
        "} as const;\nfunction findLocal(): string | undefined {\n  const custom = \
         join(process.cwd(), 'src', 'routes.ts');\n  if (existsSync(custom)) { return \
         process.cwd() + '/src/routes'; }\n}\nexport function loadRoutes(additional: (keyof \
         typeof ROUTES)[] = []) {\n  const routePaths = new Set<string>();\n  const localPath = \
         findLocal();\n  if (localPath) { routePaths.add(localPath); }\n  for (const name of \
         additional) {\n    if (!ROUTES[name]) { throw new Error(name); }\n    \
         routePaths.add(ROUTES[name]);\n  }\n  let merged: unknown[] = [];\n  for (const \
         routePath of routePaths) {\n    const loaded = require(routePath);\n    merged = \
         [...merged, ...loaded.routes];\n  }\n  return { routes: merged };\n}\n",
    );
    let out = show(&body);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert_eq!(lines[0], "fs_read cwd:src/routes.ts !cwd_dependent/cwd");
    // The cwd-relative path keeps its base and a literal specifier (the per-test
    // working directory layer resolves it); the 18 table values stay exact
    // alternatives.
    assert_eq!(lines[1], "require cwd:./src/routes !cwd_dependent/cwd");
    assert!(
        lines[2]
            .starts_with("require dir:services/svc-a-0/src/routes | services/svc-a-1/src/routes"),
        "{out}"
    );
    assert_eq!(lines[2].matches(" | ").count(), 17, "{out}");
    assert!(!lines[2].contains('!'), "{out}");
}

// ---- bounds ----

#[test]
fn unparseable_files_keep_the_token_results() {
    // The token result stays, with an open record: nothing checked the name `d`.
    assert_eq!(
        show("const d = join(__dirname, 'x');\nreadFileSync(join(d, 'a'));\nfunction {\n"),
        "fs_read dir:x/a !computed_specifier/unknown"
    );
    // A literal needs no check.
    assert_eq!(
        show("readFileSync('/etc/hosts');\nrequire('./x');\nfunction {\n"),
        "fs_read dir:/etc/hosts\nrequire dir:./x"
    );
}

#[test]
fn deep_nesting_is_not_parsed() {
    let deep = format!(
        "const x = {}1{};\nfunction f() {{ const d = join(__dirname, 'd'); return \
         readFileSync(d); }}\n",
        "(".repeat(600),
        ")".repeat(600)
    );
    assert_eq!(
        show(&deep),
        "fs_read dir:<unknown> !computed_specifier/unknown"
    );
    let chain = format!(
        "const y = c{};\nfunction f() {{ const d = join(__dirname, 'd'); return readFileSync(d); \
         }}\n",
        " ? a : c".repeat(4000)
    );
    assert_eq!(
        show(&chain),
        "fs_read dir:<unknown> !computed_specifier/unknown"
    );
}

#[test]
fn many_sites_stay_linear() {
    let mut body = String::from("const d = join(__dirname, 'd');\n");
    for i in 0..3000 {
        body.push_str(&format!(
            "function f{i}(p) {{ const q = join(d, p); return readFileSync(q); }}\nf{i}('a{i}');\n"
        ));
    }
    let start = std::time::Instant::now();
    let out = show_as(&body, Dialect::JS, "main.js");
    assert!(
        out.starts_with("fs_read dir:d/a0\nfs_read dir:d/a1\n"),
        "{}",
        &out[..80]
    );
    assert!(start.elapsed().as_secs() < 20);
}

// ---- next to a resolved module, and resolved from another place ----

const MODULE: &str = "import { createRequire } from 'node:module';\n";

/// A file next to a module that `import.meta.resolve` / `require.resolve`
/// finds: the URL and path functions keep the module, through bindings and
/// local functions.
#[test]
fn next_to_a_resolved_module() {
    assert_eq!(
        show(
            "const root = new URL('.', import.meta.resolve('eve'));\nfunction at(rel: string) { \
             return new URL(rel, root).href; }\nawait import(at('inner.js'));\nawait \
             import(at(process.env.X ? 'a.js' : 'b/c.js'));\n"
        ),
        "dynamic_import dir:eve\ndynamic_import \
         mod:import.meta.resolve('eve')/../inner.js\ndynamic_import \
         mod:import.meta.resolve('eve')/../a.js | import.meta.resolve('eve')/../b/c.js"
    );
    assert_eq!(
        show(&format!(
            "{MODULE}const require = createRequire(import.meta.url);\nconst root = \
             dirname(require.resolve('eve/package.json'));\nawait import(pathToFileURL(join(root, \
             'dist', 'x.js')).href);\nreadFileSync(join(root, 'README.md'));\nconst entry = \
             pathToFileURL(require.resolve('eve')).href;\nawait import(new URL('./y.js', \
             entry).pathname);\n"
        )),
        "require dir:eve/package.json\ndynamic_import \
         mod:require.resolve('eve/package.json')/../dist/x.js\nfs_read \
         mod:require.resolve('eve/package.json')/../README.md\nrequire dir:eve\ndynamic_import \
         mod:require.resolve('eve')/../y.js"
    );
    // A parameter of an exported function: a pattern below the directory, and the
    // record of a partial specifier.
    assert_eq!(
        show(
            "const root = new URL('dist/', \
             pathToFileURL(require.resolve('eve/package.json')));\nexport const load = (p: \
             string) => import(new URL(p, root).href);\n"
        ),
        "require dir:eve/package.json\ndynamic_import \
         mod:require.resolve('eve/package.json')/../dist/* !partial_specifier/unknown"
    );
    // Text appended without a `/` is not a path below the module.
    assert_eq!(
        show("await import(dirname(require.resolve('eve')) + '-other/x.js');\n"),
        "dynamic_import dir:<unknown> !computed_specifier/unknown\nrequire dir:eve"
    );
}

/// `createRequire(x)` as a value: bindings, containers and local functions decide where
/// the calls resolve from; what cannot be evaluated is unknown, never the file.
// rustfmt's `format_strings` would change this item's string literals.
#[rustfmt::skip]
#[test]
fn create_require_is_a_value() {
    let m = |body: &str| show(&format!("{MODULE}{body}"));
    assert_eq!(
        m(
            "const anchors = { other: join(__dirname, '../other/package.json') };\n\
           const req = createRequire(anchors.other);\n\
           req('./x.js');\nreq('dep');\nreq.resolve('dep/sub');\n"
        ),
        "require dir:../other/x.js\n\
         require mod:require.resolve('dep', from '../other')\n\
         require mod:require.resolve('dep/sub', from '../other')"
    );
    // A URL object, a `file:` string and a directory with its `/`.
    assert_eq!(
        m(
            "createRequire(new URL('../other/a.js', import.meta.url))('./x.js');\n\
           createRequire(pathToFileURL(join(__dirname, '../other/a.js')).href)('./y.js');\n\
           createRequire(join(__dirname, '../other') + '/')('./z.js');\n\
           createRequire(new URL('../other/', import.meta.url))('./w.js');\n"
        ),
        "require dir:../other/x.js\nrequire dir:../other/y.js\nrequire dir:../other/z.js\n\
         require dir:../other/w.js"
    );
    // The file itself, however it is written.
    assert_eq!(
        m("let req = createRequire(fileURLToPath(import.meta.url));\n\
           req('./a.js');\n\
           createRequire(join(__dirname, 'package.json'))('dep');\n"),
        "require dir:./a.js\nrequire dir:dep"
    );
    // Nested: a `require` for a module found through another one.
    assert_eq!(
        m(
            "const eve = createRequire(import.meta.url).resolve('eve/package.json');\n\
           const inner = createRequire(createRequire(eve).resolve('dep'));\n\
           inner('./lib/x.js');\ninner('leaf');\n"
        ),
        "require dir:eve/package.json\n\
         require mod:require.resolve('dep', from require.resolve('eve/package.json')/'..')\n\
         require mod:require.resolve('dep', from require.resolve('eve/package.json')/'..')/../lib/x.js\n\
         require mod:require.resolve('leaf', from require.resolve('dep', from require.resolve('eve/package.json')/'..')/'..')"
    );
    // Not evaluated: a parameter, the working directory, a pattern, a function's result.
    for anchor in [
        "dir",
        "process.cwd() + '/'",
        "join(__dirname, name, 'package.json')",
        "find()",
    ] {
        let out = m(&format!(
            "export function f(dir: string, name: string) {{ return createRequire({anchor})('./x.js'); }}\n"
        ));
        assert!(
            out.starts_with("require dir:<unknown> !computed_specifier/") && !out.contains('\n'),
            "{anchor}: {out}"
        );
    }
    // A `require` function passed along is the one that was passed.
    assert_eq!(
        m(
            "const other = createRequire(join(__dirname, '../other/a.js'));\n\
           function load(other: NodeRequire) { return other('./x.js'); }\n\
           load(require);\n"
        ),
        "require dir:./x.js"
    );
}

/// `require.resolve(request, options)` in the engine: the entries of `paths`,
/// through bindings; anything else in their place is unknown.
#[test]
fn resolve_options() {
    assert_eq!(
        show(
            "const options = { paths: [join(__dirname, '../other'), \
             dirname(require.resolve('eve'))] };\nconst found = require.resolve('dep', \
             options);\nrequire(found);\n"
        ),
        "require dir:eve\nrequire mod:require.resolve('dep', from '../other') | \
         require.resolve('dep', from require.resolve('eve')/'..')\nrequire dir:<unknown> \
         !computed_specifier/unknown\nrequire mod:require.resolve('dep', from '../other') | \
         require.resolve('dep', from require.resolve('eve')/'..')\nrequire dir:<unknown> \
         !computed_specifier/call"
    );
    // (The record of the `resolve` call itself has no origin that another file's
    // value could answer: the load is asked of its callee. The `require` of its
    // result is a value like any other.)
    // (An object held in a binding and handed to a call is not trusted to stay as
    // it was written: the entries are found, and a record stays. Written in
    // place it is exact.)
    assert_eq!(
        show("require(require.resolve('dep', { paths: [join(__dirname, '../other')] }));\n"),
        "require mod:require.resolve('dep', from '../other')\nrequire mod:require.resolve('dep', \
         from '../other')"
    );
    // The options are written elsewhere too: unknown as well.
    let out = show(
        "const options = { paths: [join(__dirname, '../other')] };\nexport function set(p: \
         string[]) { options.paths = p; }\nrequire.resolve('dep', options);\n",
    );
    assert!(
        out.contains("require dir:<unknown> !computed_specifier/"),
        "{out}"
    );
    // An entry that is a file searches from its directory (the resolver decides);
    // an array with unknown elements is unknown.
    assert_eq!(
        show(
            "require.resolve('dep', { paths: [require.resolve('eve/package.json')] });\nexport \
             const f = (extra: string[]) => require.resolve('dep', { paths: [...extra] });\n"
        ),
        "require mod:require.resolve('dep', from require.resolve('eve/package.json'))\nrequire \
         dir:eve/package.json\nrequire dir:<unknown> !computed_specifier/unknown"
    );
}

/// A load asked of a `require` function made for a place the engine cannot
/// evaluate keeps its record, and the record names no origin that a binder
/// could answer (`param`, `call`, `imported`), whatever makes the place
/// unknown: the facts make such a site no sink, and a file whose only records
/// are of this kind is no binding site. No environment template stands for it
/// either: none of these has an alternative that could be evaluated.
#[test]
fn a_load_from_an_unknown_place_names_no_origin() {
    let unknown = "require dir:<unknown> !computed_specifier/unknown";
    for (what, body) in [
        (
            "the environment",
            "createRequire(join(process.env.OTHER, 'package.json')).resolve('dep/package.json');\n",
        ),
        (
            "a call",
            "import { root } from './root';\nconst req = createRequire(join(root(), \
             'package.json'));\nreq('dep');\n",
        ),
        (
            "an imported binding",
            "import { ROOT } from './root';\ncreateRequire(join(ROOT, 'package.json'))('dep');\n",
        ),
        (
            "a parameter",
            "export function load(dir: string) { return createRequire(dir + \
             '/package.json')('dep'); }\n",
        ),
    ] {
        let src = format!("{MODULE}{body}");
        assert_eq!(show(&src), unknown, "{what}");
        let whole = format!("{HEAD}{src}");
        let ex = scan_named(
            whole.as_bytes(),
            Dialect::TS,
            ScanOptions::default(),
            Some("main.ts"),
        );
        assert!(
            !ex.facts
                .iter()
                .any(|f| matches!(f, turborepo_trace_core::env::EnvFact::Use { .. })),
            "{what}: no template"
        );
    }
    // The file's own `require` with a value from elsewhere is a value like any
    // other.
    assert_eq!(
        show("export function load(name: string) { return require(name); }\n"),
        "require dir:<unknown> !computed_specifier/param"
    );
}

/// The references of `MODULE + body` ([`show`]) and its environment templates,
/// one per line: `use <origin of the record>/<where the load searches from>`.
fn loads(body: &str) -> String {
    use turborepo_trace_core::env::EnvFact;
    let src = format!("{MODULE}{body}");
    let mut out = vec![show(&src)];
    let whole = format!("{HEAD}{src}");
    let ex = scan_named(
        whole.as_bytes(),
        Dialect::TS,
        ScanOptions::default(),
        Some("main.ts"),
    );
    for f in &ex.facts {
        if let EnvFact::Use {
            origin,
            from,
            shared,
            ..
        } = f
        {
            out.push(format!(
                "use {}/{from:?}{}",
                turborepo_trace_core::Origin::ALL[*origin as usize].as_str(),
                if *shared { " shared" } else { "" }
            ));
        }
    }
    out.join("\n")
}

/// A load asked of a `require` function whose request comes from the
/// environment keeps its template, and the template says where the function
/// searches from: only a relative value depends on that, and the environment
/// layer resolves it from there (or not at all, where the place is not known).
#[test]
fn a_load_from_the_environment_says_where_it_searches() {
    let open = "require dir:<unknown> !env_dependent/env";
    // A name that can be bound again: asked of its callee, which is the file's own.
    assert_eq!(
        loads("let req = createRequire(import.meta.url);\nreq(process.env.PLUGIN);\n"),
        format!("{open}\nuse env/File")
    );
    // A computed place in the file's directory.
    assert_eq!(
        loads("const req = createRequire(join(__dirname, 'noop.js'));\nreq(process.env.PLUGIN);\n"),
        format!("{open}\nuse env/File")
    );
    // A known place in another directory.
    assert_eq!(
        loads(
            "const req = createRequire(join(__dirname, '..', 'other', \
             'noop.js'));\nreq(process.env.PLUGIN);\n"
        ),
        format!("{open}\nuse env/Dir(\"../other\")")
    );
    // A place that is not known: the variable is still what is asked for.
    assert_eq!(
        loads(
            "const req = createRequire(join(process.cwd(), \
             'noop.js'));\nreq(process.env.PLUGIN);\n"
        ),
        format!("{open}\nuse env/Unknown")
    );
    // Next to another request, which the unknown place leaves unknown: two records,
    // and the template stands for the one of the variable.
    assert_eq!(
        loads(
            "const req = createRequire(join(process.cwd(), 'noop.js'));\nreq(process.env.PLUGIN \
             || './default');\n"
        ),
        format!("require dir:<unknown> !computed_specifier/unknown\n{open}\nuse env/Unknown")
    );
    // A path resolved where it is asked is the same for every `require` function:
    // its template is anchored at the working directory, whatever the place.
    assert_eq!(
        loads(
            "const req = createRequire(join(process.cwd(), \
             'noop.js'));\nreq(resolve(process.cwd(), process.env.PLUGIN));\n"
        ),
        format!("{open}\nuse env/File")
    );
}

/// The request is the result of an imported function (which may be an
/// environment helper). Asked of the file's own `require` function it has a
/// template; the record names no origin a binder could answer, and the template
/// says the origin its record ends with. Where another alternative of the load
/// has a record of the same kind at the same place, the template says that its
/// record is shared: resolving it must not close the other. From a place that
/// is not known the call has no template.
#[test]
fn a_load_of_what_an_imported_function_returns() {
    let own = "import { pluginPath } from './env';\nconst req = createRequire(join(__dirname, \
               'noop.js'));\n";
    assert_eq!(
        loads(&format!("{own}req(pluginPath());\n")),
        "require dir:<unknown> !computed_specifier/unknown\nuse unknown/File"
    );
    assert_eq!(
        loads(&format!(
            "{own}export function load(p: string, c: boolean) {{ return req(c ? pluginPath() : \
             p); }}\n"
        )),
        "require dir:<unknown> !computed_specifier/unknown\nrequire dir:<unknown> \
         !computed_specifier/unknown\nuse unknown/File shared"
    );
    assert_eq!(
        loads(
            "import { pluginPath } from './env';\nconst req = createRequire(join(process.cwd(), \
             'noop.js'));\nreq(pluginPath());\n"
        ),
        "require dir:<unknown> !computed_specifier/unknown"
    );
}
