//! Tests of fact extraction and binding. The fixtures keep the shapes of real
//! sites (a token reader behind a two-hop chain ending in the environment, a
//! seeding helper that reads a field of each element of an array argument, a
//! class that reads a constructor argument, a callback of an imported cache
//! wrapper) with other names.

use std::path::Path;

use super::{file_facts, show};

pub(super) const READER: &str = r#"import { readFile } from 'fs/promises';

export function newProvider({ reader, offset }: { reader: () => Promise<string>; offset: number }) {
  let token: string | undefined;
  return async () => {
    token = await reader();
    return token;
  };
}

export function newFileReader(path: string) {
  return async function readFromPath() {
    const text = await readFile(path, 'ascii');
    return parse(text);
  };
}

export function parse(text: string) {
  return text.trim();
}
"#;

pub(super) const ACCOUNT: &str = r#"import { newProvider, newFileReader } from './reader';

export interface Options {
  tokenFile?: string;
}

export function accountProvider(opts?: Options) {
  const tokenFile = opts?.tokenFile ?? process.env['TOKEN_FILE'];
  if (!tokenFile) {
    throw new Error('token file not specified');
  }
  return newProvider({ reader: newFileReader(tokenFile), offset: 5 });
}
"#;

pub(super) const SEED: &str = r#"import fs from 'fs';

interface Table {
  name: string;
  seedFilePath?: string;
}

export async function seedTables(client: { exec(sql: string): Promise<void> }, tables: Table[]) {
  for (const table of tables) {
    if (!table.seedFilePath) continue;
    const sql = fs.readFileSync(table.seedFilePath, 'utf8');
    await client.exec(sql);
  }
}
"#;

pub(super) const SEED_CALLER: &str = r#"import path from 'path';
import { seedTables } from '@scope/db';

const tables = [
  { name: 'events', seedFilePath: path.join(__dirname, 'seeds/events.sql') },
  { name: 'plain' },
];

beforeAll(async () => {
  await seedTables(globalThis.client, tables);
});
"#;

pub(super) const USES: &str = r#"import { f, g, h } from './m';
import * as ns from './n';
import type { T } from './t';
import def from './d';

f('a');
const x: ReturnType<typeof g> | T = undefined as any;
[1].map(h);
ns.k('z');
new def.Thing(1);
export { f as renamed };
export * from './star';
export { y as z } from './named';
"#;

pub(super) const CONFIG_MAP: &str = r#"import fs from 'fs';

export class ConfigMap {
  private readonly path: string;

  constructor(path: string) {
    this.path = path;
  }

  load() {
    return fs.readFileSync(this.path, 'utf8');
  }
}

export const local = new ConfigMap('/etc/config/local.json');
"#;

pub(super) const WRAPPED: &str = r#"import { promises as fsp } from 'fs';
import { wrapCache } from '@scope/wrap';

const cache = new Map();

const listDir = wrapCache(cache, (dir: string) => {
  return fsp.readdir(dir);
});

async function getKey(keysPath: string) {
  const names = await listDir(keysPath);
  return names[0];
}

export default getKey;
"#;

pub(super) const WRAP: &str = r#"export function wrapCache<T extends any[], V>(
  cache: Map<unknown, Promise<V>>,
  fn: (...values: T) => Promise<V>,
): (...values: T) => Promise<V> {
  return (...values: T) => {
    const key = values[0];
    let p = cache.get(key);
    if (!p) {
      p = fn(...values);
      cache.set(key, p);
    }
    return p;
  };
}
"#;

pub(super) const ROUTES: &str = r#"import { existsSync } from 'fs';
import { join } from 'path';

const ROUTES_BY_SERVICE = {
  'svc-a': 'services/svc-a/src/routes',
  'svc-b': 'services/svc-b/src/routes.ts',
};

export type ServiceName = keyof typeof ROUTES_BY_SERVICE;

function localRoutes(): string | undefined {
  const custom = join(process.cwd(), 'src', 'routes.ts');
  if (existsSync(custom)) {
    return process.cwd() + '/src/routes';
  }
}

export function loadRoutes(extra: ServiceName[] = []) {
  const paths = new Set<string>();
  const local = localRoutes();
  if (local) {
    paths.add(local);
  }
  for (const name of extra) {
    if (!ROUTES_BY_SERVICE[name]) {
      throw new Error(`Invalid value: ${name}`);
    }
    paths.add(ROUTES_BY_SERVICE[name]);
  }
  let merged: unknown[] = [];
  for (const routePath of paths) {
    const loaded = require(routePath);
    merged = [...merged, ...loaded.routes];
  }
  return { routes: merged };
}

interface Opts {
  extraRoutes?: ServiceName[];
  seed?: boolean;
}

export function setupEndpoint(opts: Opts = {}) {
  const { extraRoutes = [] } = opts;
  const definition = loadRoutes(extraRoutes);
  return { definition, seed: opts.seed ?? true };
}
"#;

#[test]
fn table_read_by_a_key_from_callers() {
    let f = facts(ROUTES, "setup-endpoint.ts");
    println!("{f}");
    assert!(f.contains("sink require:32 (resolved)"), "{f}");
    assert!(f.contains("[setupEndpoint#0.extraRoutes[]]"), "{f}");
    assert!(f.contains("[loadRoutes#0[]]"), "{f}");
    assert!(f.contains("table 2 value(s) keyed only"), "{f}");
    assert!(f.contains("fn loadRoutes: named returns"), "{f}");
    assert!(f.contains("import fs\n"), "{f}");
}

fn facts(src: &str, name: &str) -> String {
    match file_facts(src.as_bytes(), Path::new(name)) {
        Some(f) => show::facts(&f),
        None => "none".to_owned(),
    }
}

#[test]
fn reader_facts() {
    let f = facts(READER, "reader.ts");
    println!("{f}");
    assert!(f.contains("export newFileReader = fn newFileReader"), "{f}");
    assert!(f.contains("fn newFileReader: named"), "{f}");
    assert!(f.contains("sink fs_read:13 newFileReader#0"), "{f}");
}

#[test]
fn caller_with_env_and_own_parameter() {
    let f = facts(ACCOUNT, "index.ts");
    println!("{f}");
    assert!(f.contains("import ./reader\n"), "{f}");
    assert!(
        f.contains(
            "call:12 import(./reader#newFileReader)(accountProvider#0.tokenFile | ?env(env \
             TOKEN_FILE))"
        ),
        "{f}"
    );
    assert!(f.contains("fn accountProvider: named"), "{f}");
}

#[test]
fn field_of_each_element() {
    let f = facts(SEED, "seed-tables.ts");
    println!("{f}");
    assert!(
        f.contains("sink fs_read:11 seedTables#1[].seedFilePath"),
        "{f}"
    );
    let c = facts(SEED_CALLER, "setup.ts");
    println!("{c}");
    assert!(
        c.contains(
            "import(@scope/db#seedTables)(?unknown, [{name: \"events\", seedFilePath: \
             <dir>/seeds/events.sql}, {name: \"plain\"}"
        ),
        "{c}"
    );
}

#[test]
fn uses_of_imported_bindings() {
    let f = facts(USES, "uses.ts");
    println!("{f}");
    // `h` is handed to `map`: the file does more than read and call it.
    assert!(f.contains("import ./m escapes h writes\n"), "{f}");
    assert!(f.contains("import ./n\n"), "{f}");
    assert!(!f.contains("import ./t"), "{f}");
    assert!(f.contains("import ./d\n"), "{f}");
    assert!(f.contains("reexport ./m f as renamed"), "{f}");
    assert!(f.contains("reexport ./star *"), "{f}");
    assert!(f.contains("reexport ./named y as z"), "{f}");
    assert!(f.contains("import(./m#f)(\"a\")"), "{f}");
    assert!(f.contains("import(./n#k)(\"z\")"), "{f}");
    assert!(f.contains("new import(./d#default.Thing)(\"1\")"), "{f}");
}

#[test]
fn constructor_parameter() {
    let f = facts(CONFIG_MAP, "config-map.ts");
    println!("{f}");
    assert!(f.contains("export ConfigMap = fn constructor"), "{f}");
    assert!(f.contains("fn constructor: ctor\n"), "{f}");
    assert!(f.contains("sink fs_read:11 constructor#0"), "{f}");
    assert!(
        f.contains("new fn constructor(\"/etc/config/local.json\")"),
        "{f}"
    );
}

#[test]
fn callback_of_an_imported_wrapper() {
    let f = facts(WRAPPED, "get-key.ts");
    println!("{f}");
    assert!(f.contains("export default = fn getKey"), "{f}");
    assert!(
        f.contains("callback 1 of import(@scope/wrap#wrapCache)"),
        "{f}"
    );
    assert!(f.contains("sink fs_read_dir:7"), "{f}");
    let w = facts(WRAP, "wrap.ts");
    println!("{w}");
    assert!(w.contains("export wrapCache = fn wrapCache"), "{w}");
    // The returned closure forwards its arguments to the callback.
    assert!(w.contains("wrapCache#1(...function#0[])"), "{w}");
}

#[test]
fn facts_survive_the_cache_format() {
    use super::codec::{decode, encode};
    for (src, name) in [
        (READER, "reader.ts"),
        (ACCOUNT, "index.ts"),
        (SEED, "seed-tables.ts"),
        (SEED_CALLER, "setup.ts"),
        (USES, "uses.ts"),
        (CONFIG_MAP, "config-map.ts"),
        (WRAPPED, "get-key.ts"),
        (WRAP, "wrap.ts"),
        (ROUTES, "setup-endpoint.ts"),
    ] {
        let f = file_facts(src.as_bytes(), Path::new(name)).expect(name);
        let bytes = encode(&f);
        assert_eq!(decode(&bytes).as_ref(), Some(&f), "{name}");
        // Damaged or foreign data is rejected, never misread.
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None, "{name}");
        let mut other = bytes.clone();
        other[0] ^= 0x40;
        assert_eq!(decode(&other), None, "{name}");
    }
    assert_eq!(decode(&[]), None);
}

#[test]
fn shared_values_do_not_unfold_exponentially() {
    // Each step uses the previous value twice: 40 steps are 2^40 leaves as a tree.
    let mut src = String::from("import { f } from './m';\nexport function g(v0: string) {\n");
    for i in 1..=40 {
        src.push_str(&format!("  const v{i} = v{} + '/' + v{};\n", i - 1, i - 1));
    }
    src.push_str("  return f(v40);\n}\n");
    let t = std::time::Instant::now();
    let f = facts(&src, "deep.ts");
    assert!(t.elapsed().as_secs() < 5, "{:?}", t.elapsed());
    // The argument is too large to describe: it ends in an unknown, which keeps
    // what depends on it open, and the facts stay small.
    assert!(f.contains("import(./m#f)("), "{f}");
    assert!(f.contains("?unknown"), "{f}");
    assert!(f.len() < 20_000, "{}", f.len());
}

#[test]
fn unanalyzable_files_have_no_facts() {
    assert_eq!(facts("with (x) { y }", "a.js"), "none");
    assert_eq!(facts("export const a = eval('1');", "a.js"), "none");
    assert_eq!(facts("export const = ;", "a.ts"), "none");
}

// ---- binding ----

mod replay;

use std::sync::Arc;

use super::{
    bind::{Importer, Select, World, bind},
    facts::FileFacts,
};

/// A repository in memory: files by path, with relative and aliased imports
/// resolved by name.
struct Repo {
    files: Vec<(String, Option<Arc<FileFacts>>)>,
    aliases: Vec<(&'static str, &'static str)>,
    /// `(importer, target)`: loads that are not static declarations.
    dynamic: Vec<(&'static str, &'static str)>,
    /// Working directories of tests.
    cwds: Vec<&'static str>,
}

impl Repo {
    fn new(files: &[(&str, &str)]) -> Repo {
        Repo {
            files: files
                .iter()
                .map(|(p, src)| {
                    (
                        (*p).to_owned(),
                        file_facts(src.as_bytes(), Path::new(p)).map(Arc::new),
                    )
                })
                .collect(),
            aliases: Vec::new(),
            dynamic: Vec::new(),
            cwds: Vec::new(),
        }
    }

    fn id(&self, path: &str) -> Option<u32> {
        self.files
            .iter()
            .position(|(p, _)| p == path)
            .map(|i| i as u32)
    }

    fn resolve(&self, from: u32, spec: &str) -> Option<u32> {
        if let Some((_, to)) = self.aliases.iter().find(|(a, _)| *a == spec) {
            return self.id(to);
        }
        let rel = spec.strip_prefix("./")?;
        let from = &self.files[from as usize].0;
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        let base = if dir.is_empty() {
            rel.to_owned()
        } else {
            format!("{dir}/{rel}")
        };
        self.id(&format!("{base}.ts")).or_else(|| self.id(&base))
    }

    fn spec_at(&self, file: u32, at: u32) -> Option<String> {
        let f = self.files[file as usize].1.as_ref()?;
        f.imports
            .iter()
            .filter(|i| i.at == at)
            .map(|i| i.spec.clone())
            .chain(
                f.reexports
                    .iter()
                    .filter(|r| r.at == at)
                    .map(|r| r.spec.clone()),
            )
            .next()
    }

    fn bound(&self, roots: &[&str], select: Select) -> String {
        let roots: Vec<u32> = roots.iter().filter_map(|r| self.id(r)).collect();
        let b = bind(self, &roots, select);
        show::bound(&b, &|f| self.files[f as usize].0.clone())
    }
}

impl World for Repo {
    fn path(&self, file: u32) -> String {
        self.files[file as usize].0.clone()
    }

    fn facts(&self, file: u32) -> Option<Arc<FileFacts>> {
        self.files[file as usize].1.clone()
    }

    fn targets(&self, file: u32, at: u32) -> Vec<u32> {
        self.spec_at(file, at)
            .and_then(|s| self.resolve(file, &s))
            .into_iter()
            .collect()
    }

    fn resolve(&self, file: u32, spec: &str) -> Vec<u32> {
        Repo::resolve(self, file, spec).into_iter().collect()
    }

    fn module_at(&self, path: &str) -> Vec<u32> {
        self.id(path)
            .or_else(|| self.id(&format!("{path}.ts")))
            .into_iter()
            .collect()
    }

    fn cwds(&self) -> Vec<String> {
        self.cwds.iter().map(|c| (*c).to_owned()).collect()
    }

    fn file(&self, path: &str) -> Option<u32> {
        self.id(path)
    }

    fn importers(&self, file: u32) -> Vec<Importer> {
        let mut out = Vec::new();
        for (g, (_, facts)) in self.files.iter().enumerate() {
            let g = g as u32;
            let Some(facts) = facts else { continue };
            let mut ats: Vec<u32> = facts.imports.iter().map(|i| i.at).collect();
            ats.extend(facts.reexports.iter().map(|r| r.at));
            ats.sort_unstable();
            ats.dedup();
            for at in ats {
                if self.targets(g, at).contains(&file) {
                    out.push(Importer {
                        file: g,
                        at: Some(at),
                    });
                }
            }
        }
        for (g, t) in &self.dynamic {
            if self.id(t) == Some(file) {
                out.push(Importer {
                    file: self.id(g).unwrap_or(0),
                    at: None,
                });
            }
        }
        out
    }
}

const LOADS: Select = Select {
    loads: true,
    reads: false,
};
const ALL: Select = Select {
    loads: true,
    reads: true,
};

fn routes_repo(extra: &[(&'static str, &'static str)]) -> Repo {
    let mut files = vec![
        ("helpers/setup-endpoint.ts", ROUTES),
        (
            "pkg/a.test.ts",
            "import { setupEndpoint } from '@scope/helpers';\nconst { client } = setupEndpoint({ \
             extraRoutes: ['svc-a'] });\n",
        ),
        (
            "pkg/b.test.ts",
            "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint();\n",
        ),
        ("helpers/index.ts", "export * from './setup-endpoint';\n"),
    ];
    files.extend_from_slice(extra);
    let mut r = Repo::new(&files);
    r.aliases.push(("@scope/helpers", "helpers/index.ts"));
    r
}

#[test]
fn a_key_selects_the_table_values_at_the_caller() {
    let r = routes_repo(&[]);
    let b = r.bound(&["helpers/setup-endpoint.ts"], LOADS);
    println!("{b}");
    assert_eq!(
        b,
        "closed helpers/setup-endpoint.ts resolved [services/svc-a/src/routes, \
         services/svc-b/src/routes.ts]\ndep pkg/a.test.ts:2 require spec \
         services/svc-a/src/routes <- helpers/setup-endpoint.ts:32\ntable \
         helpers/setup-endpoint.ts [services/svc-a/src/routes, services/svc-b/src/routes.ts]"
    );
}

#[test]
fn an_unknown_key_selects_every_value_at_that_caller() {
    let r = routes_repo(&[(
        "tools/lifecycle.ts",
        "import { loadRoutes } from '@scope/helpers';\nexport const routes = \
         loadRoutes([process.argv[2]]);\n",
    )]);
    let b = r.bound(&["helpers/setup-endpoint.ts"], LOADS);
    println!("{b}");
    assert!(
        b.contains("closed helpers/setup-endpoint.ts resolved"),
        "{b}"
    );
    assert!(
        b.contains("dep pkg/a.test.ts:2 require spec services/svc-a/src/routes <-"),
        "{b}"
    );
    assert!(
        !b.contains("dep pkg/a.test.ts:2 require spec services/svc-b"),
        "{b}"
    );
    // A key from the command line selects every value, at the file that passes it.
    assert!(
        b.contains("dep tools/lifecycle.ts:2 require spec services/svc-a/src/routes <-"),
        "{b}"
    );
    assert!(
        b.contains("dep tools/lifecycle.ts:2 require spec services/svc-b/src/routes.ts <-"),
        "{b}"
    );
}

#[test]
fn an_escaping_use_is_bound_where_it_escapes() {
    let r = routes_repo(&[(
        "pkg/c.test.ts",
        "import { setupEndpoint } from '@scope/helpers';\nconst variants = \
         [setupEndpoint];\nvariants.forEach((s) => s());\n",
    )]);
    let b = r.bound(&["helpers/setup-endpoint.ts"], LOADS);
    println!("{b}");
    assert!(
        b.contains("closed helpers/setup-endpoint.ts resolved"),
        "{b}"
    );
    assert!(
        b.contains("dep pkg/c.test.ts:0 require spec services/svc-a/src/routes <-"),
        "{b}"
    );
    assert!(
        b.contains("dep pkg/c.test.ts:0 require spec services/svc-b/src/routes.ts <-"),
        "{b}"
    );
    assert!(
        !b.contains("dep pkg/a.test.ts:2 require spec services/svc-b"),
        "{b}"
    );
}

#[test]
fn a_load_the_facts_do_not_describe_is_bound_at_the_loader() {
    let mut r = routes_repo(&[(
        "pkg/d.test.ts",
        "const h = require('@scope/helpers');\nh.setupEndpoint({ extraRoutes: ['svc-b'] });\n",
    )]);
    r.dynamic.push(("pkg/d.test.ts", "helpers/index.ts"));
    let b = r.bound(&["helpers/setup-endpoint.ts"], LOADS);
    println!("{b}");
    // The barrel is loaded with `require`: whoever does it may call anything with
    // anything.
    assert!(
        b.contains("dep pkg/d.test.ts:0 require spec services/svc-a/src/routes <-"),
        "{b}"
    );
    assert!(
        b.contains("closed helpers/setup-endpoint.ts resolved"),
        "{b}"
    );
}

#[test]
fn an_escape_inside_the_callee_keeps_it_open() {
    let src = ROUTES.replace(
        "interface Opts {",
        "export const registry = [loadRoutes];\n\ninterface Opts {",
    );
    let mut r = Repo::new(&[
        ("helpers/setup-endpoint.ts", &src),
        (
            "pkg/a.test.ts",
            "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint({ extraRoutes: \
             ['svc-a'] });\n",
        ),
    ]);
    r.aliases
        .push(("@scope/helpers", "helpers/setup-endpoint.ts"));
    let b = r.bound(&["helpers/setup-endpoint.ts"], LOADS);
    println!("{b}");
    assert!(!b.contains("closed"), "{b}");
    assert!(!b.contains("table"), "{b}");
    assert!(
        b.contains("dep pkg/a.test.ts:2 require spec services/svc-a/src/routes"),
        "{b}"
    );
}

const ENTRYPOINT: &str = r#"interface Options {
  configPath: string;
  tags: string[];
}

export async function generateEntrypoint(options: Options) {
  const content = await generateSource(options, '/tmp');
  return { content };
}

export async function generateSource(options: Options, moduleDirectory: string) {
  const config = require(options.configPath);
  return JSON.stringify({ config, moduleDirectory, tags: options.tags });
}
"#;

#[test]
fn a_module_path_parameter_is_bound_to_its_callers() {
    let mut r = Repo::new(&[
        ("framework/generate.ts", ENTRYPOINT),
        (
            "framework/generate.test.ts",
            "import { join } from 'path';\nimport { generateEntrypoint } from \
             './generate';\nit('works', async () => {\n  await generateEntrypoint({ configPath: \
             join(__dirname, 'fixtures/routes.ts'), tags: [] });\n});\n",
        ),
        (
            "checks/entries.ts",
            "import { generateEntrypoint } from '@scope/framework';\nexport async function \
             entries(service: { routesPath: string }) {\n  return generateEntrypoint({ \
             configPath: service.routesPath, tags: [] });\n}\n",
        ),
    ]);
    r.aliases
        .push(("@scope/framework", "framework/generate.ts"));
    let b = r.bound(&["framework/generate.ts"], LOADS);
    println!("{b}");
    assert!(b.contains("closed framework/generate.ts record"), "{b}");
    assert!(
        b.contains(
            "dep framework/generate.test.ts:4 require path framework/fixtures/routes.ts <- \
             framework/generate.ts:12"
        ),
        "{b}"
    );
    // Nothing calls `entries`: it adds nothing.
    assert!(!b.contains("checks/entries.ts"), "{b}");
    // Once something loads its module in a way the facts do not describe, the
    // unknown argument is an open record of that loader.
    r.dynamic
        .push(("framework/generate.test.ts", "checks/entries.ts"));
    let b = r.bound(&["framework/generate.ts"], LOADS);
    println!("{b}");
    assert!(b.contains("closed framework/generate.ts record"), "{b}");
    assert!(
        b.contains("dep framework/generate.test.ts:0 require open param"),
        "{b}"
    );
}

#[test]
fn reads_are_bound_only_when_selected() {
    let mut r = Repo::new(&[
        ("lib/reader.ts", READER),
        ("lib/index.ts", ACCOUNT),
        (
            "app/main.js",
            "const { accountProvider } = require('../lib');\n",
        ),
    ]);
    r.dynamic.push(("app/main.js", "lib/index.ts"));
    assert_eq!(r.bound(&["lib/reader.ts"], LOADS), "");
    let b = r.bound(&["lib/reader.ts"], ALL);
    println!("{b}");
    assert!(b.contains("closed lib/reader.ts record"), "{b}");
    // The environment alternative belongs to the caller that reads it.
    assert!(b.contains("dep lib/index.ts:12 fs_read open env"), "{b}");
    // The option comes from whoever loads the module with `require`.
    assert!(b.contains("dep app/main.js:0 fs_read open param"), "{b}");
}

#[test]
fn field_of_each_element_is_bound() {
    let mut r = Repo::new(&[("db/seed-tables.ts", SEED), ("svc/setup.ts", SEED_CALLER)]);
    r.aliases.push(("@scope/db", "db/seed-tables.ts"));
    let b = r.bound(&["db/seed-tables.ts"], ALL);
    println!("{b}");
    assert!(b.contains("closed db/seed-tables.ts record"), "{b}");
    assert!(
        b.contains("dep svc/setup.ts:10 fs_read path svc/seeds/events.sql <- db/seed-tables.ts:11"),
        "{b}"
    );
}

#[test]
fn constructor_and_exported_constant() {
    let mut r = Repo::new(&[
        ("lib/config-map.ts", CONFIG_MAP),
        (
            "lib/paths.ts",
            "import { join } from 'path';\nexport const CONFIG = join(__dirname, 'data', \
             'config.json');\n",
        ),
        (
            "app/main.ts",
            "import { ConfigMap } from '@scope/config-map';\nimport { CONFIG } from \
             '@scope/paths';\nexport const map = new ConfigMap(CONFIG);\n",
        ),
    ]);
    r.aliases.push(("@scope/config-map", "lib/config-map.ts"));
    r.aliases.push(("@scope/paths", "lib/paths.ts"));
    let b = r.bound(&["lib/config-map.ts"], ALL);
    println!("{b}");
    assert!(b.contains("closed lib/config-map.ts record"), "{b}");
    assert!(
        b.contains("dep app/main.ts:3 fs_read path lib/data/config.json <- lib/config-map.ts:11"),
        "{b}"
    );
    // `/etc/config/local.json` (the `new` in the file itself) is outside the
    // repository.
    assert!(!b.contains("etc/config"), "{b}");
}

#[test]
fn return_summaries_and_constants_of_other_files() {
    let mut r = Repo::new(&[
        (
            "lib/paths.ts",
            "import { join } from 'path';\nexport function configPath(name: string) {\n  return \
             join(__dirname, 'configs', `${name}.js`);\n}\nexport const DEFAULT = join(__dirname, \
             'configs', 'default.js');\nexport function nothing() {}\n",
        ),
        (
            "app/load.ts",
            "import { configPath, DEFAULT, nothing } from '@scope/paths';\nimport { external } \
             from 'somewhere';\nexport const a = require(configPath('prod'));\nexport const b = \
             require(DEFAULT);\nexport const c = require(external('x'));\nexport const d = \
             require(nothing());\n",
        ),
    ]);
    r.aliases.push(("@scope/paths", "lib/paths.ts"));
    let b = r.bound(&["app/load.ts"], LOADS);
    println!("{b}");
    assert!(
        b.contains("dep app/load.ts:3 require path lib/configs/prod.js <- app/load.ts:3"),
        "{b}"
    );
    assert!(
        b.contains("dep app/load.ts:4 require path lib/configs/default.js <- app/load.ts:4"),
        "{b}"
    );
    // The call of a function of a module without facts stays the file's own record;
    // a function that returns nothing loads nothing.
    let load = r.id("app/load.ts").unwrap();
    let bound = bind(&r, &[load], LOADS);
    assert_eq!(bound.closed.len(), 3, "{b}");
    assert!(!b.contains("app/load.ts:5"), "{b}");
    // The same values for a consumer with its own sinks.
    let facts = r.facts(r.id("app/load.ts").unwrap()).unwrap();
    let call = facts
        .calls
        .iter()
        .find(|c| c.line == 3)
        .expect("the call of configPath");
    let vals = super::bind::values(&r, r.id("app/load.ts").unwrap(), &call.args[0]);
    assert_eq!(vals.len(), 1);
    assert_eq!(show::js(&vals[0].value), "\"prod\"");
}

#[test]
fn thousands_of_callers_with_distinct_options() {
    // Every caller passes a different options object; the option the table is read
    // with has two values.
    let mut files: Vec<(String, String)> = vec![
        ("helpers/setup-endpoint.ts".to_owned(), ROUTES.to_owned()),
        (
            "helpers/index.ts".to_owned(),
            "export * from './setup-endpoint';\n".to_owned(),
        ),
    ];
    for i in 0..1500 {
        let svc = if i % 2 == 0 { "svc-a" } else { "svc-b" };
        files.push((
            format!("pkg/t{i}.test.ts"),
            format!(
                "import {{ setupEndpoint }} from '@scope/helpers';\nsetupEndpoint({{ extraRoutes: \
                 ['{svc}'], seed: false, tag: 't{i}' }});\n"
            ),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut r = Repo::new(&refs);
    r.aliases.push(("@scope/helpers", "helpers/index.ts"));
    let root = r.id("helpers/setup-endpoint.ts").unwrap();
    let b = bind(&r, &[root], LOADS);
    assert!(!b.exhausted);
    let opens: Vec<_> = b
        .deps
        .iter()
        .filter(|d| matches!(d.value, super::bind::DepValue::Open { .. }))
        .take(3)
        .collect();
    assert_eq!(b.closed.len(), 1, "deps {} opens {:?}", b.deps.len(), opens);
    assert_eq!(b.tables.len(), 1);
    // One dependency per caller: the module its key selects.
    assert_eq!(b.deps.len(), 1500);
    let t7 = r.id("pkg/t7.test.ts").unwrap();
    let of_t7: Vec<_> = b.deps.iter().filter(|d| d.file == t7).collect();
    assert_eq!(of_t7.len(), 1);
    assert_eq!(
        of_t7[0].value,
        super::bind::DepValue::Spec("services/svc-b/src/routes.ts".to_owned())
    );
}

#[test]
fn too_many_combinations_keep_the_site_open() {
    // Three parameters with 40 values each: the callers pass 40 triples, a join of
    // the parameters is 64,000 combinations. Nothing is dropped silently: the
    // site's own record stays.
    let mut files: Vec<(String, String)> = vec![(
        "lib/load.ts".to_owned(),
        "import { join } from 'path';\nexport function load(a: string, b: string, c: string) {\n  \
         return require(join(a, b, c));\n}\n"
            .to_owned(),
    )];
    for i in 0..40 {
        files.push((
            format!("pkg/c{i}.ts"),
            format!(
                "import {{ load }} from '@scope/load';\nexport const m = load(__dirname, 'a{i}', \
                 'm{i}.js');\n"
            ),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut r = Repo::new(&refs);
    r.aliases.push(("@scope/load", "lib/load.ts"));
    let root = r.id("lib/load.ts").unwrap();
    let b = bind(&r, &[root], LOADS);
    assert!(b.closed.is_empty());
    // With few callers the site closes; each combination is some caller's
    // dependency.
    let mut r = Repo::new(&refs[..4]);
    r.aliases.push(("@scope/load", "lib/load.ts"));
    let b = bind(&r, &[root], LOADS);
    assert_eq!(b.closed.len(), 1);
    assert_eq!(b.deps.len(), 27);
    assert!(
        b.deps
            .iter()
            .any(|d| d.value == super::bind::DepValue::Path("pkg/a1/m1.js".to_owned()))
    );
}

/// The shape of an endpoint harness and a framework: the harness loads the
/// route modules a key selects and the one of the working directory, and hands
/// their tables to the framework, which loads every handler they name.
const HARNESS: &str = r#"import { fromConfig } from '@scope/framework';

const ROUTES_BY_SERVICE = {
  'svc-a': 'services/svc-a/src/routes',
  'svc-b': 'services/svc-b/src/routes',
};

export function setupEndpoint(opts: { extraRoutes?: string[] } = {}) {
  const { extraRoutes = [] } = opts;
  const paths = new Set<string>();
  paths.add(process.cwd() + '/src/routes');
  for (const name of extraRoutes) {
    paths.add(ROUTES_BY_SERVICE[name]);
  }
  let merged: unknown[] = [];
  for (const routePath of paths) {
    const loaded = require(routePath);
    merged = [...merged, ...loaded.routes];
  }
  return fromConfig({ routes: merged });
}
"#;

const FRAMEWORK: &str = r#"function normalize(spec: any) {
  return () => import(spec);
}

export function fromConfig(config: { routes: any[] }) {
  return config.routes.map((group) => group.handler.map((h: any) => normalize(h.handler)));
}
"#;

fn harness_repo() -> Repo {
    let mut r = Repo::new(&[
        ("helpers/setup-endpoint.ts", HARNESS),
        ("framework/index.ts", FRAMEWORK),
        (
            "services/svc-a/src/routes.ts",
            "module.exports = {\n  routes: [{ tracing: true, handler: [{ method: 'GET', handler: \
             'services/svc-a/src/endpoints/get' }] }],\n};\n",
        ),
        (
            "services/svc-b/src/routes.ts",
            "module.exports = {\n  routes: [{ handler: [{ method: 'GET', handler: \
             'services/svc-b/src/endpoints/list' }] }],\n};\n",
        ),
        (
            "services/svc-c/src/routes.ts",
            "const prefix = process.argv[2];\nmodule.exports = {\n  routes: [{ handler: [{ \
             method: 'GET', handler: prefix + '/endpoints/x' }] }],\n};\n",
        ),
        (
            "pkg/a.test.ts",
            "import { setupEndpoint } from '@scope/helpers';\nsetupEndpoint({ extraRoutes: \
             ['svc-a'] });\n",
        ),
    ]);
    r.aliases
        .push(("@scope/helpers", "helpers/setup-endpoint.ts"));
    r.aliases.push(("@scope/framework", "framework/index.ts"));
    r.aliases
        .push(("services/svc-a/src/routes", "services/svc-a/src/routes.ts"));
    r.aliases
        .push(("services/svc-b/src/routes", "services/svc-b/src/routes.ts"));
    r
}

#[test]
fn a_bound_require_is_followed_into_the_module() {
    let f = facts(HARNESS, "setup-endpoint.ts");
    println!("{f}");
    assert!(f.contains("require("), "{f}");
    let m = facts(
        "module.exports = { routes: [{ handler: 'a/b' }] };\nexports.extra = 'c/d';\n",
        "routes.ts",
    );
    println!("{m}");
    assert!(
        m.contains("export module.exports = {routes: [{handler: \"a/b\"}]}"),
        "{m}"
    );
    assert!(m.contains("export extra = \"c/d\""), "{m}");

    let mut r = harness_repo();
    let b = r.bound(&["framework/index.ts"], LOADS);
    println!("{b}");
    // The framework's site is closed: the handler a key selects is a literal of the
    // route module, which supplies it.
    assert!(b.contains("closed framework/index.ts record"), "{b}");
    assert!(
        b.contains(
            "dep services/svc-a/src/routes.ts:0 dynamic_import spec \
             services/svc-a/src/endpoints/get <- framework/index.ts:2"
        ),
        "{b}"
    );
    assert!(!b.contains("svc-b/src/endpoints"), "{b}");
    // The route module of the working directory: evaluated for the tests' working
    // directories; the record on the harness does not count for tests.
    assert!(
        b.contains("dep helpers/setup-endpoint.ts:20 dynamic_import cwd-per-test"),
        "{b}"
    );
    assert!(!b.contains(" open "), "{b}");

    // A test that runs in `services/svc-b`, and one in `services/svc-c`, whose
    // table has a handler that is not a literal: that module has the open
    // record.
    r.cwds = vec!["services/svc-b", "services/svc-c", "pkg"];
    let b = r.bound(&["framework/index.ts"], LOADS);
    println!("{b}");
    assert!(b.contains("closed framework/index.ts record"), "{b}");
    assert!(
        b.contains(
            "dep services/svc-b/src/routes.ts:0 dynamic_import spec \
             services/svc-b/src/endpoints/list <-"
        ),
        "{b}"
    );
    assert!(
        b.contains("dep services/svc-c/src/routes.ts:0 dynamic_import open argv"),
        "{b}"
    );
}

// ---- soundness review: nothing that is given up may close a site ----

const LOAD_BY_NAME: &str = "export function load(name: string) {\n  return require(name);\n}\n";

fn caller(import: &str, call: &str) -> String {
    format!("import {{ {import} }} from './helper';\n{call};\n")
}

#[test]
fn a_derived_value_is_unknown_at_every_caller() {
    // Some function of the argument: every caller supplies an unknown, not only
    // one.
    let a = caller("load", "load('./mods/A')");
    let b = caller("load", "load('./mods/B')");
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function load(name: string) {\n  return require(name.toLowerCase());\n}\n",
        ),
        ("lib/a.test.ts", &a),
        ("lib/b.test.ts", &b),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("dep lib/a.test.ts:2 require open"), "{out}");
    assert!(out.contains("dep lib/b.test.ts:2 require open"), "{out}");
}

#[test]
fn an_explicit_reexport_wins_over_a_star() {
    let t = caller("load", "load()");
    let r = Repo::new(&[
        ("lib/a.ts", "export const DIR = './wrong';\n"),
        ("lib/b.ts", "export const DIR = './right';\n"),
        (
            "lib/barrel.ts",
            "export * from './a';\nexport { DIR } from './b';\n",
        ),
        (
            "lib/helper.ts",
            "import { DIR } from './barrel';\nexport function load() {\n  return \
             require(DIR);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("require spec ./right"), "{out}");
    assert!(!out.contains("./wrong"), "{out}");
}

#[test]
fn every_assignment_of_an_export_counts() {
    let t = caller("load", "load()");
    let r = Repo::new(&[
        (
            "lib/dirs.ts",
            "exports.dir = './one';\nif (process.env.OTHER) {\n  exports.dir = './two';\n}\n",
        ),
        (
            "lib/helper.ts",
            "import { dir } from './dirs';\nexport function load() {\n  return require(dir);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("require spec ./one"), "{out}");
    assert!(out.contains("require spec ./two"), "{out}");
}

#[test]
fn barrels_that_reexport_each_other() {
    let r = Repo::new(&[
        ("lib/f.ts", LOAD_BY_NAME),
        ("lib/a.ts", "export * from './f';\nexport * from './b';\n"),
        ("lib/b.ts", "export * from './a';\n"),
        (
            "lib/c.test.ts",
            "import { load } from './a';\nload('./mods/c');\n",
        ),
        (
            "lib/d.test.ts",
            "import { load } from './b';\nload('./mods/d');\n",
        ),
    ]);
    let out = r.bound(&["lib/f.ts"], LOADS);
    println!("{out}");
    assert!(
        out.contains("dep lib/c.test.ts:2 require spec ./mods/c"),
        "{out}"
    );
    assert!(
        out.contains("dep lib/d.test.ts:2 require spec ./mods/d"),
        "{out}"
    );
    assert!(out.contains("closed lib/f.ts record"), "{out}");
}

const LOAD_PLUGIN_OF: &str =
    "export function load(p: string) {\n  const m = require(p);\n  return require(m.plugin);\n}\n";

#[test]
fn a_module_outside_the_repository_exports_something_unknown() {
    let t = caller("load", "load('/etc/app/config.js')");
    let r = Repo::new(&[("lib/helper.ts", LOAD_PLUGIN_OF), ("lib/t.test.ts", &t)]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    // What the module names as its plugin is not known: an open record of the
    // caller.
    assert!(out.contains("dep lib/t.test.ts:2 require open"), "{out}");
}

#[test]
fn the_file_that_loads_a_module_is_a_caller_of_its_exports() {
    // `plugin.ts` is only loaded through the bound site of `helper.ts`: the graph
    // has no edge for it, but `helper.ts` calls its export with an argument.
    let t = caller("run", "run('./plugin')");
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function run(p: string) {\n  const m = require(p);\n  return \
             m.setup('./mods/c');\n}\n",
        ),
        (
            "lib/plugin.ts",
            "export function setup(dir: string) {\n  return require(dir + '/index');\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts", "lib/plugin.ts"], LOADS);
    println!("{out}");
    assert!(
        out.contains("dep lib/t.test.ts:2 require spec ./plugin"),
        "{out}"
    );
    // The argument of `setup` is not known: the loader holds the module and may
    // call anything with anything.
    assert!(
        out.contains("dep lib/helper.ts:0 require open param"),
        "{out}"
    );
}

#[test]
fn a_table_value_that_leaves_another_way_keeps_the_table() {
    let t = caller("load", "load('a')");
    let table = "const T = { a: './mods/a', b: './mods/b' };\nexport function load(key: 'a' | \
                 'b') {\n  return require(T[key]);\n}\n";
    let leaky = format!("{table}export function pathOf(key: 'a' | 'b') {{\n  return T[key];\n}}\n");
    let r = Repo::new(&[("lib/helper.ts", table), ("lib/t.test.ts", &t)]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("table lib/helper.ts"), "{out}");
    // The same table, also read by a function that returns the value: whoever gets
    // it may load it, so the references of the literal have to stay.
    let r = Repo::new(&[("lib/helper.ts", &leaky), ("lib/t.test.ts", &t)]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("table lib/helper.ts"), "{out}");
    assert!(
        out.contains("dep lib/t.test.ts:2 require spec ./mods/a"),
        "{out}"
    );
}

#[test]
fn an_object_the_callee_changes_is_not_the_callers() {
    let t = caller("load", "load({ name: './mods/a' })");
    // Written in the reading file.
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function load(opts: { name: string }) {\n  opts.name = './mods/b';\n  return \
             require(opts.name);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
    // Handed to a function of another file that writes the property.
    let helper = "import { prepare } from './prepare';\nexport function load(opts: { name?: \
                  string }) {\n  prepare(opts);\n  return require(opts.name);\n}\n";
    let r = Repo::new(&[
        ("lib/helper.ts", helper),
        (
            "lib/prepare.ts",
            "export function prepare(o: { name?: string }) {\n  o.name = o.name || \
             './mods/b';\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
    // Handed to a function that hands it on to one that writes it.
    let r = Repo::new(&[
        ("lib/helper.ts", helper),
        (
            "lib/prepare.ts",
            "import { fill } from './fill';\nexport function prepare(o: { name?: string }) {\n  \
             return fill(o);\n}\n",
        ),
        (
            "lib/fill.ts",
            "export function fill(o: { name?: string }) {\n  o.name = './mods/b';\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
    // Handed to a function that only looks at it: the caller's value stands.
    let r = Repo::new(&[
        ("lib/helper.ts", helper),
        (
            "lib/prepare.ts",
            "export function prepare(o: { name?: string }) {\n  return typeof o.name === \
             'string';\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("closed lib/helper.ts record"), "{out}");
    assert!(
        out.contains("dep lib/t.test.ts:2 require spec ./mods/a"),
        "{out}"
    );
    // Handed to something that is not a function with facts.
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "import { prepare } from 'some-package';\nexport function load(opts: { name?: string \
             }) {\n  prepare(opts);\n  return require(opts.name);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
}

#[test]
fn exports_another_file_can_change_are_unknown_where_that_file_ran() {
    let t = caller("load", "load('./config')");
    let files = |other: &'static str| {
        Repo::new(&[
            ("lib/helper.ts", LOAD_PLUGIN_OF),
            (
                "lib/config.ts",
                "module.exports = { plugin: './mods/a' };\n",
            ),
            ("lib/t.test.ts", &t),
            ("lib/other.test.ts", other),
        ])
    };
    let r = files("import cfg from './config';\ncfg.plugin = './mods/z';\n");
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    // The literal of the module is supplied by the module...
    assert!(
        out.contains("dep lib/config.ts:0 require spec ./mods/a"),
        "{out}"
    );
    // ...and whatever the other file made of it by the file that did.
    assert!(
        out.contains("dep lib/other.test.ts:0 require open"),
        "{out}"
    );
    let r = files("import cfg from './config';\nconsole.log(cfg.plugin);\n");
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(
        out.contains("dep lib/config.ts:0 require spec ./mods/a"),
        "{out}"
    );
    assert!(!out.contains("lib/other.test.ts"), "{out}");
}

#[test]
fn exports_written_in_a_form_that_is_not_listed_are_open() {
    let t = caller("load", "load('./config')");
    let r = Repo::new(&[
        ("lib/helper.ts", LOAD_PLUGIN_OF),
        (
            "lib/config.ts",
            "module.exports = { plugin: './mods/a' };\nconst e = module.exports;\ne.plugin = \
             process.env.PLUGIN;\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("dep lib/config.ts:0 require open"), "{out}");
}

#[test]
fn a_partly_unknown_alternative_keeps_the_record() {
    let t = caller("load", "load('./mods/a', false)");
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function load(name: string, own: boolean) {\n  return require(own ? \
             process.env.PLUGIN_DIR + '/x' : name);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    // The record stays, so every test that reaches the helper is selected on every
    // change: what the callers pass adds nothing and is not evaluated.
    assert_eq!(out, "");
}

#[test]
fn what_is_not_a_string_at_a_module_load() {
    // `import()` takes the string form of its argument.
    let helper = "export function load(spec: unknown) {\n  return import(spec as string);\n}\n";
    let bound = |call: &str| {
        let t = caller("load", call);
        let r = Repo::new(&[("lib/helper.ts", helper), ("lib/t.test.ts", &t)]);
        let out = r.bound(&["lib/helper.ts"], LOADS);
        println!("{call}:\n{out}");
        out
    };
    assert!(bound("load(['./mods/a'])").contains("dynamic_import spec ./mods/a"));
    assert!(!bound("load(['./mods/a', './mods/b'])").contains("dep "));
    assert!(!bound("load({ a: 1 })").contains("dep "));
    assert!(!bound("load(() => './mods/a')").contains("dep "));
    assert!(
        bound("load({ toString() { return './mods/a'; } })")
            .contains("dep lib/t.test.ts:2 dynamic_import open")
    );
    // `require` of anything but a string throws.
    let helper = "export function load(spec: any) {\n  return require(spec);\n}\n";
    let t = caller("load", "load(['./mods/a'])");
    let r = Repo::new(&[("lib/helper.ts", helper), ("lib/t.test.ts", &t)]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("dep "), "{out}");
    // A character of a string is a string: `.` names the directory's index.
    let helper = "export function load(spec: string) {\n  return require(spec[0]);\n}\n";
    let t = caller("load", "load('./mods/a')");
    let r = Repo::new(&[("lib/helper.ts", helper), ("lib/t.test.ts", &t)]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("dep lib/t.test.ts:2 require open"), "{out}");
}

#[test]
fn a_method_can_change_its_receiver_a_function_read_from_it_cannot() {
    // `o.fix()` runs the caller's method with the object as `this`.
    let t = caller(
        "load",
        "load({ name: './mods/a', fix() { this.name = './mods/b'; } })",
    );
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function load(o: { name: string; fix(): void }) {\n  o.fix();\n  return \
             require(o.name);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
    // A hook taken out of the options and called on its own gets no `this`.
    let t = caller(
        "load",
        "load({ name: './mods/a', before: (f: () => void) => f() })",
    );
    let r = Repo::new(&[
        (
            "lib/helper.ts",
            "export function load(opts: { name: string; before?: (f: () => void) => void }) {\n  \
             const { before = queueMicrotask } = opts;\n  before(() => {});\n  return \
             require(opts.name);\n}\n",
        ),
        ("lib/t.test.ts", &t),
    ]);
    let out = r.bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("closed lib/helper.ts record"), "{out}");
    assert!(
        out.contains("dep lib/t.test.ts:2 require spec ./mods/a"),
        "{out}"
    );
}

const NORMALIZE: &str = r#"import { isFunction, isString } from './util';

export function normalize({ spec, merge }: any): any[] {
  if (Array.isArray(spec)) {
    return spec.map((item: any) => normalize({ spec: item, merge })).flat();
  }
  if (isFunction(spec)) {
    return [{ handler: spec }];
  }
  if (isString(spec)) {
    return [{ handler: () => import(spec), modulePath: spec }];
  }
  return normalize({ spec: spec.handler, merge }).map(merge(spec));
}
"#;

const UTIL: &str = "export function isFunction(x: unknown) {\n  return typeof x === \
                    'function';\n}\nexport function isString(x: unknown) {\n  return typeof x === \
                    'string';\n}\n";

#[test]
fn a_function_that_walks_its_argument_by_calling_itself() {
    let f = facts(NORMALIZE, "normalize.ts");
    println!("{f}");
    // The binding that the recursion fills is everything its reads reach, not
    // unknown.
    assert!(f.contains("reach(normalize#0"), "{f}");
    let test = "import { normalize } from '../framework/normalize';\nconst merge = (s: any) => \
                (h: any) => ({ ...s, ...h });\nnormalize({ spec: [{ tracing: true, handler: [{ \
                method: 'GET', path: '/x', handler: './handlers/get', tags: ['public'] }, { \
                method: 'POST', path: '/x', handler: [{ handler: './handlers/post' }] }, { \
                handler: () => 1 }] }], merge });\n";
    let repo = |util: &str, extra: &[(&str, &str)]| {
        let mut files = vec![
            ("framework/normalize.ts", NORMALIZE),
            ("framework/util.ts", util),
            ("app/a.test.ts", test),
        ];
        files.extend_from_slice(extra);
        let mut r = Repo::new(&files);
        r.aliases
            .push(("../framework/normalize", "framework/normalize.ts"));
        r
    };
    let out = repo(UTIL, &[]).bound(&["framework/normalize.ts"], LOADS);
    println!("{out}");
    // The handlers at every depth, and nothing else of the table: no method, path
    // or tag, no object, array or function.
    assert_eq!(
        out,
        "closed framework/normalize.ts record\ndep app/a.test.ts:3 dynamic_import spec \
         ./handlers/get <- framework/normalize.ts:11\ndep app/a.test.ts:3 dynamic_import spec \
         ./handlers/post <- framework/normalize.ts:11"
    );
    // A caller whose value is not known keeps its own record.
    let out = repo(
        UTIL,
        &[(
            "app/b.test.ts",
            "import { normalize } from '../framework/normalize';\nnormalize({ spec: \
             JSON.parse(process.env.ROUTES!), merge: (s: any) => (h: any) => h });\n",
        )],
    )
    .bound(&["framework/normalize.ts"], LOADS);
    println!("{out}");
    assert!(
        out.contains("closed framework/normalize.ts record"),
        "{out}"
    );
    assert!(
        out.contains("dep app/b.test.ts:2 dynamic_import open"),
        "{out}"
    );
    assert!(
        out.contains("dep app/a.test.ts:3 dynamic_import spec ./handlers/get"),
        "{out}"
    );
    // A helper that changes what it is handed makes the walk unknown.
    let out = repo(
        "export function isFunction(x: any) {\n  x.handler = './handlers/other';\n  return \
         false;\n}\nexport function isString(x: unknown) {\n  return typeof x === 'string';\n}\n",
        &[],
    )
    .bound(&["framework/normalize.ts"], LOADS);
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
}

/// `{ ...rest, name: v }`: a property written by name after the last spread is
/// `v`, whatever the spread brings. Written before the spread, the spread can
/// replace it.
#[test]
fn a_property_written_after_a_spread_is_what_was_written() {
    let helper =
        "export function load(opts: { name: string }) {\n  return require(opts.name);\n}\n";
    let caller = |object: &str| {
        format!(
            "import {{ load }} from './helper';\nconst base = \
             JSON.parse(process.env.BASE!);\nload({object});\n"
        )
    };
    let bound = |object: &str| {
        Repo::new(&[
            ("lib/helper.ts", helper),
            ("lib/caller.ts", &caller(object)),
        ])
        .bound(&["lib/helper.ts"], LOADS)
    };
    let out = bound("{ ...base, name: './mods/a' }");
    println!("{out}");
    assert_eq!(
        out,
        "closed lib/helper.ts record\ndep lib/caller.ts:3 require spec ./mods/a <- lib/helper.ts:2"
    );
    // The spread comes last: it can bring its own `name`.
    let out = bound("{ name: './mods/a', ...base }");
    println!("{out}");
    assert!(out.contains("dep lib/caller.ts:3 require open"), "{out}");
    assert!(
        out.contains("dep lib/caller.ts:3 require spec ./mods/a"),
        "{out}"
    );
    // A second spread after the property can replace it.
    let out = bound("{ ...base, name: './mods/a', ...base }");
    println!("{out}");
    assert!(out.contains("dep lib/caller.ts:3 require open"), "{out}");
    // A computed key after it can be the property: its value is one more
    // alternative, and what the first spread brings still is not.
    let out = bound("{ ...base, name: './mods/a', [process.env.KEY!]: './mods/b' }");
    println!("{out}");
    assert_eq!(
        out,
        "closed lib/helper.ts record\ndep lib/caller.ts:3 require spec ./mods/a <- \
         lib/helper.ts:2\ndep lib/caller.ts:3 require spec ./mods/b <- lib/helper.ts:2"
    );
    // A property that is not written is whatever the spread brings.
    let out = bound("{ ...base, other: './mods/a' }");
    println!("{out}");
    assert!(out.contains("dep lib/caller.ts:3 require open"), "{out}");
    assert!(!out.contains("./mods/a"), "{out}");
}

// ---- the floor of the route normalizer: what keeps a walk over route tables
// open ----

/// A method called on the result of an imported function (`load(x).filter(f)`)
/// is no hidden use of the import: the call itself is recorded, and a function
/// the module hands out in its result has its unknown callers at home.
#[test]
fn a_method_of_a_call_result_is_no_escape() {
    let helper = "export function many(specs: string[]) {\n  return specs.map((s) => \
                  require(s));\n}\nexport function load(spec: string) {\n  return \
                  require(spec);\n}\nexport function api() {\n  return { go: load };\n}\n";
    let bound = |import: &str, call: &str| {
        Repo::new(&[
            ("lib/helper.ts", helper),
            ("lib/t.test.ts", &caller(import, call)),
        ])
        .bound(&["lib/helper.ts"], LOADS)
    };
    let out = bound("many", "many(['./mods/a']).filter(Boolean)");
    println!("{out}");
    assert!(
        out.contains("dep lib/t.test.ts:2 require spec ./mods/a"),
        "{out}"
    );
    assert!(!out.contains("open"), "{out}");
    // Two methods in a row, and a method with a callback.
    let out = bound("many", "many(['./mods/a']).map((m) => m).filter(Boolean)");
    println!("{out}");
    assert!(!out.contains("open"), "{out}");
    // A member of the binding itself is a call the facts do not show.
    let out = bound("many", "many.call(null, ['./mods/a'])");
    println!("{out}");
    assert!(out.contains("dep lib/t.test.ts:0 require open"), "{out}");
    // A function of the module reached through a result: `load` is used as a value
    // in its own file, so its site stays open there whatever the caller does.
    let out = bound("api", "api().go('./mods/b')");
    println!("{out}");
    assert!(!out.contains("./mods/b"), "{out}");
    assert_eq!(out.matches("closed").count(), 1, "{out}");
}

/// A load behind a test that its argument is a string takes strings only: the
/// arrays and objects a walk passes through do not reach it.
#[test]
fn a_site_behind_a_test_for_a_string_takes_strings_only() {
    let is = "export function isString(v: unknown): v is string {\n  return typeof v === \
              'string';\n}\nexport const arrow = (v: unknown) => typeof v === 'string';\nexport \
              function loose(v: any) {\n  return typeof v === 'string' || !!v.ok;\n}\nexport \
              function other(v: any, w: any) {\n  return typeof w === 'string';\n}\nexport async \
              function later(v: any) {\n  return typeof v === 'string';\n}\nexport function \
              rebound(v: any) {\n  v = String(v);\n  return typeof v === 'string';\n}\n";
    let facts = show::facts(&file_facts(is.as_bytes(), Path::new("is.ts")).unwrap());
    println!("{facts}");
    assert!(
        facts.contains("fn isString: named") && facts.contains("tests #0 for a string"),
        "{facts}"
    );
    // `other` tests its second parameter; the others are no such test.
    assert!(
        facts.contains("fn other: named returns ?unknown tests #1"),
        "{facts}"
    );
    assert_eq!(facts.matches("for a string").count(), 3, "{facts}");
    let helper = |test: &str| {
        format!(
            "import {{ isString, arrow, loose, other, later, rebound }} from './is';\nexport \
             function load(spec: any): any {{\n  if (Array.isArray(spec)) return spec.map((s) => \
             load(s));\n  if ({test}) return import(spec);\n  return load(spec.handler);\n}}\n"
        )
    };
    let t = caller("load", "load([{ handler: './mods/a' }, [['./mods/b']]])");
    let bound = |test: &str| {
        Repo::new(&[
            ("lib/helper.ts", &helper(test)),
            ("lib/is.ts", is),
            ("lib/t.test.ts", &t),
        ])
        .bound(&["lib/helper.ts"], LOADS)
    };
    for test in [
        "isString(spec)",
        "typeof spec === 'string'",
        "'string' == typeof spec",
        "arrow(spec)",
        "spec.length > 0 && isString(spec)",
    ] {
        let out = bound(test);
        println!("{test}:\n{out}");
        assert!(out.contains("closed lib/helper.ts record"), "{test}: {out}");
        assert!(
            out.contains("spec ./mods/a") && out.contains("spec ./mods/b"),
            "{test}: {out}"
        );
        assert!(!out.contains("open"), "{test}: {out}");
    }
    // Not such a test: the nested array reaches the load (the string form of
    // `[['x']]` is `x`).
    for test in [
        "loose(spec)",
        "other(spec, 'x')",
        "later(spec)",
        "rebound(spec)",
        "!isString(spec)",
        "spec.ok || isString(spec)",
        "isString(spec.handler)",
        "typeof spec !== 'string'",
        "true",
    ] {
        let out = bound(test);
        println!("{test}:\n{out}");
        assert!(
            out.contains("dep lib/t.test.ts:2 dynamic_import open"),
            "{test}: {out}"
        );
    }
    // The binding is assigned behind the test, or the load is not inside the
    // branch.
    for body in [
        "  if (isString(spec)) {\n    spec = alt;\n    return import(spec);\n  }\n",
        "  if (isString(spec)) console.log(spec);\n  return import(spec);\n",
        "  if (isString(spec)) return null;\n  else return import(spec);\n",
        "  if (isString(alt)) return import(spec);\n",
    ] {
        let helper = format!(
            "import {{ isString }} from './is';\nexport function load(spec: any, alt?: any): any \
             {{\n{body}}}\n"
        );
        let out = Repo::new(&[
            ("lib/helper.ts", &helper),
            ("lib/is.ts", is),
            (
                "lib/t.test.ts",
                &caller("load", "load([['./mods/b']], [['./mods/c']])"),
            ),
        ])
        .bound(&["lib/helper.ts"], LOADS);
        println!("{body}:\n{out}");
        assert!(out.contains("dynamic_import open"), "{body}: {out}");
    }
    // A function made behind the test still sees the string.
    let lazy = "import { isString } from './is';\nexport function lazy(spec: any) {\n  if \
                (isString(spec)) return () => import(spec);\n  return () => null;\n}\n";
    let out = Repo::new(&[
        ("lib/helper.ts", lazy),
        ("lib/is.ts", is),
        (
            "lib/t.test.ts",
            &caller("lazy", "lazy([['./mods/b']]);\nlazy('./mods/a')"),
        ),
    ])
    .bound(&["lib/helper.ts"], LOADS);
    println!("{out}");
    assert!(out.contains("closed lib/helper.ts record"), "{out}");
    assert!(
        out.contains("spec ./mods/a") && !out.contains("open"),
        "{out}"
    );
}

/// An array method hands the elements of its receiver, and what they hold, to
/// its callback: a callback the caller supplies can change them before the
/// helper reads them.
#[test]
fn an_array_method_hands_its_elements_to_a_callback() {
    let each = "export function each(list: { name: string }[], fix: (o: any) => void) {\n  \
                list.forEach(fix);\n  return require(list[0].name);\n}\n";
    let wrap = "export function each(spec: { name: string }, fix: (o: any) => void) {\n  [{ \
                inner: spec }].map(fix);\n  return require(spec.name);\n}\n";
    let bound = |helper: &str, call: &str| {
        Repo::new(&[
            ("lib/helper.ts", helper),
            ("lib/t.test.ts", &caller("each", call)),
        ])
        .bound(&["lib/helper.ts"], LOADS)
    };
    // The callback writes the property: the caller's literal is not all there is.
    let out = bound(
        each,
        "each([{ name: './mods/a' }], (it) => {\n  it.name = process.env.NAME;\n})",
    );
    println!("{out}");
    assert!(out.contains("dep lib/t.test.ts:2 require open"), "{out}");
    let out = bound(
        wrap,
        "each({ name: './mods/a' }, (w) => {\n  w.inner.name = process.env.NAME;\n})",
    );
    println!("{out}");
    assert!(out.contains("dep lib/t.test.ts:2 require open"), "{out}");
    // A callback that writes nothing leaves it alone.
    for helper in [each, wrap] {
        let arg = if helper == each {
            "[{ name: './mods/a' }]"
        } else {
            "{ name: './mods/a' }"
        };
        let out = bound(helper, &format!("each({arg}, () => {{}})"));
        println!("{out}");
        assert_eq!(
            out,
            "closed lib/helper.ts record\ndep lib/t.test.ts:2 require spec ./mods/a <- \
             lib/helper.ts:3"
        );
    }
    // A function written in the helper is part of the helper's own writes.
    let own = "export function each(list: { name: string }[]) {\n  list.forEach((it) => {\n    \
               it.name = './mods/b';\n  });\n  return require(list[0].name);\n}\n";
    let out = bound(own, "each([{ name: './mods/a' }])");
    println!("{out}");
    assert!(!out.contains("closed"), "{out}");
    // The list is a copy, or the builtin is not a method of the list: what the
    // expression of the list mentions is handed all the same.
    for walk in [
        "[...list].forEach(fix)",
        "new Set(list).forEach(fix)",
        "Array.from(list, fix)",
        "list.filter(Boolean).map(fix)",
        "Object.values(list).some(fix)",
        "list.slice(0).sort(fix)",
        "list.reduce(fix, null)",
        "Promise.all(list).then((all: any[]) => all).then(fix)",
    ] {
        let helper = format!(
            "export function each(list: {{ name: string }}[], fix: any) {{\n  {walk};\n  return \
             require(list[0].name);\n}}\n"
        );
        let out = bound(
            &helper,
            "each([{ name: './mods/a' }], (it: any) => {\n  it.name = process.env.NAME;\n})",
        );
        println!("{walk}:\n{out}");
        assert!(
            out.contains("dep lib/t.test.ts:2 require open"),
            "{walk}: {out}"
        );
        let out = bound(&helper, "each([{ name: './mods/a' }], () => {})");
        println!("{walk}:\n{out}");
        assert_eq!(
            out,
            "closed lib/helper.ts record\ndep lib/t.test.ts:2 require spec ./mods/a <- \
             lib/helper.ts:3",
            "{walk}"
        );
    }
}
