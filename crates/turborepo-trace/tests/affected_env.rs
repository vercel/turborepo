//! `affected` on file reads and module loads whose path comes from
//! an environment variable (`turborepo_trace_store::compose::env`): the values
//! the repository assigns are dependencies of the reader, a value that is only
//! ever set outside the repository is none, and an assignment that cannot be
//! evaluated keeps the reader open.
//!
//! The shapes come from real repositories, with the names changed. `--strict`
//! counts every uncertainty record, so "nothing selected under `--strict`"
//! means the record is gone.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{path::Path, process::Command};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// The jest tests affected by `changed`, sorted.
fn affected_with(root: &Path, changed: &[&str], extra: &[&str]) -> Vec<String> {
    let mut args = vec!["affected", "--tests", "jest"];
    args.extend_from_slice(extra);
    for c in changed {
        args.push("--changed");
        args.push(c);
    }
    let o = Command::new(support::harness())
        // The hints file of these tests (`turbo trace` reads none).
        .env("TURBO_TRACE_DEBUG_HINTS", root.join("hints.toml"))
        .args(&args)
        .current_dir(root)
        .env_remove("TURBO_TRACE_DEBUG_ENV")
        .output()
        .expect("run the harness");
    assert!(
        o.status.success(),
        "harness {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let mut v: Vec<String> = String::from_utf8(o.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    v.sort();
    v
}

fn affected(root: &Path, changed: &[&str]) -> Vec<String> {
    affected_with(root, changed, &[])
}

fn strict(root: &Path, changed: &[&str]) -> Vec<String> {
    affected_with(root, changed, &["--strict"])
}

fn none() -> Vec<String> {
    Vec::new()
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"r","version":"1.0.0"}"#);
    write(r, "jest.config.js", "module.exports = {};\n");
    write(r, "unrelated.txt", "x\n");
    d
}

/// The reader of the real shape: a datafile path from the environment, through
/// a const and a guard, at module level.
const CLIENT: &str = "const { readFileSync } = require('node:fs');\nconst testDatafilePath = \
                      process.env.FLAGS_TEST_DATAFILE;\nconst testDatafile = testDatafilePath\n  \
                      ? JSON.parse(readFileSync(testDatafilePath, 'utf8'))\n  : \
                      undefined;\nmodule.exports = { testDatafile };\n";

const CLIENT_TEST: &str = "require('./client');\ntest('c', () => {});\n";

/// The helper family of the real shape, in a file of its own.
const HELPERS: &str = "export function assertEnvOr(...names: string[]): string {\n  \
    for (const name of names) {\n    const value = process.env[name];\n    \
    if (value) {\n      return value;\n    }\n  }\n  \
    throw new Error('missing');\n}\n\
    export function assertEnv(name: string): string {\n  return assertEnvOr(name);\n}\n\
    export function getEnv(name: string, fallback: string): string {\n  \
    const value = process.env[name];\n  if (!value) {\n    return fallback;\n  }\n  \
    return value;\n}\n";

/// A setup file assigns the variable to a fixture in the repository: the
/// fixture is a dependency of the reader, and so is the assignment.
#[test]
fn fixture_assigned_in_a_setup_file() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    write(r, "src/other.test.js", "test('o', () => {});\n");
    write(r, "test/fixtures/flags.json", "{}\n");
    write(r, "test/fixtures/unused.json", "{}\n");
    write(
        r,
        "test/setup.js",
        "const path = require('node:path');\nprocess.env.FLAGS_TEST_DATAFILE = \
         path.join(__dirname, 'fixtures', 'flags.json');\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    assert_eq!(
        affected(r, &["test/fixtures/flags.json"]),
        ["src/client.test.js"]
    );
    assert_eq!(affected(r, &["test/fixtures/unused.json"]), none());
    // Editing the assignment re-runs the readers.
    assert_eq!(affected(r, &["test/setup.js"]), ["src/client.test.js"]);
}

/// A variable nobody in the repository assigns, read through a helper of
/// another file with a fallback outside the repository: no dependency, and no
/// record.
#[test]
fn helper_with_an_external_fallback() {
    let d = repo();
    let r = d.path();
    write(r, "lib/env.ts", HELPERS);
    write(
        r,
        "src/token.ts",
        "import { readFile } from 'node:fs/promises';\nimport { getEnv } from \
         '../lib/env';\nconst DEFAULT_TOKEN_PATH = \
         '/var/run/secrets/widget/tokens/token';\nexport async function token() {\n  return \
         readFile(getEnv('WIDGET_OIDC_TOKEN_PATH', DEFAULT_TOKEN_PATH), 'utf8');\n}\n",
    );
    write(
        r,
        "src/token.test.ts",
        "import { token } from './token';\ntest('t', async () => { await token(); });\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    assert_eq!(affected(r, &["src/token.ts"]), ["src/token.test.ts"]);
    // The helper is a dependency like any import.
    assert_eq!(affected(r, &["lib/env.ts"]), ["src/token.test.ts"]);
}

/// The same reader when a test assigns the variable: to a temporary file
/// (outside the repository), and to a fixture next to it.
#[test]
fn helper_with_assignments_in_tests() {
    let d = repo();
    let r = d.path();
    write(r, "lib/env.ts", HELPERS);
    write(
        r,
        "src/token.ts",
        "import { readFileSync } from 'node:fs';\nimport { assertEnv } from '../lib/env';\nconst \
         tokenFile = assertEnv('WIDGET_TOKEN_FILE');\nexport const token = () => \
         readFileSync(tokenFile, 'utf8');\n",
    );
    write(
        r,
        "src/token.test.ts",
        "import { token } from './token';\nprocess.env.WIDGET_TOKEN_FILE = \
         '/tmp/widget-token';\ntest('t', () => { token(); });\n",
    );
    write(r, "src/__fixtures__/token.txt", "t\n");
    write(
        r,
        "src/fixture.test.ts",
        "import path from 'node:path';\nimport { token } from './token';\nbeforeAll(() => {\n  \
         process.env.WIDGET_TOKEN_FILE = path.join(__dirname, '__fixtures__', \
         'token.txt');\n});\ntest('t', () => { token(); });\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    // The join is global: both tests reach the reader, so both depend on the
    // fixture.
    assert_eq!(
        affected(r, &["src/__fixtures__/token.txt"]),
        ["src/fixture.test.ts", "src/token.test.ts"]
    );
}

/// An assignment that cannot be evaluated keeps the reader's record, as an
/// environment read with the reason (whatever the extractor called it: here a
/// call of a helper): the env registry owns it, not the name scan. While reads
/// from the environment are ignored by default, `--strict` shows it.
#[test]
fn unevaluable_assignment_keeps_the_record() {
    let d = repo();
    let r = d.path();
    write(r, "lib/env.ts", HELPERS);
    write(
        r,
        "src/token.ts",
        "import { readFileSync } from 'node:fs';\nimport { getEnv } from '../lib/env';\nexport \
         const token = () => readFileSync(getEnv('WIDGET_TOKEN_FILE', '/etc/widget'), 'utf8');\n",
    );
    write(
        r,
        "src/token.test.ts",
        "import { token } from './token';\ntest('t', () => { token(); });\n",
    );
    write(
        r,
        "scripts/run.ts",
        "import { spawn } from 'node:child_process';\nimport { locate } from \
         'some-package';\nspawn('node', ['server.js'], { env: { ...process.env, \
         WIDGET_TOKEN_FILE: locate() } });\n",
    );
    assert_eq!(strict(r, &["unrelated.txt"]), ["src/token.test.ts"]);
    let why = affected_with(r, &["unrelated.txt"], &["--explain", "src/token.test.ts"]).join("\n");
    assert!(why.contains("src/token.ts:3 env_dependent"), "{why}");
    // Without that assignment the record is gone.
    write(r, "scripts/run.ts", "export {};\n");
    assert_eq!(strict(r, &["unrelated.txt"]), none());
}

/// A helper that is not a template (it transforms the value) stays an unknown
/// call: the record is not the registry's, and the name scan bounds it like any
/// computed read.
#[test]
fn helper_that_is_not_a_template_stays_a_computed_read() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "lib/env.ts",
        "export function getEnv(name: string): string {\n  return decode(process.env[name] ?? \
         '');\n}\ndeclare function decode(s: string): string;\n",
    );
    write(
        r,
        "src/token.ts",
        "import { readFileSync } from 'node:fs';\nimport { getEnv } from '../lib/env';\nexport \
         const token = () => readFileSync(getEnv('WIDGET_TOKEN_FILE'), 'utf8');\n",
    );
    write(
        r,
        "src/token.test.ts",
        "import { token } from './token';\ntest('t', () => { token(); });\n",
    );
    assert_eq!(strict(r, &["unrelated.txt"]), ["src/token.test.ts"]);
}

/// The real shape of the largest site: a test server hands a variable to the
/// processes it starts, from a constructor parameter whose default is a
/// temporary file made by a helper of another file; a test constructs the
/// server with its own temporary path. The file alone cannot evaluate the
/// assignment.
#[test]
fn assignment_from_a_constructor_parameter_needs_a_cross_file_evaluator() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    write(
        r,
        "server/temp-file.ts",
        "import { randomUUID } from 'crypto';\nimport path from 'path';\n\
         export function tempDir(): string {\n  \
         return process.env.TMP_FOLDER || path.resolve(__dirname, '../.tmp');\n}\n\
         export function tempFile({\n  prefix,\n  suffix,\n}: {\n  prefix?: string;\n  suffix?: string;\n} = {}): string {\n  \
         return path.resolve(\n    tempDir(),\n    `${prefix ?? ''}${randomUUID()}${suffix ?? ''}`,\n  );\n}\n",
    );
    write(
        r,
        "server/flags.ts",
        "import { writeFile } from 'node:fs/promises';\nimport { tempFile } from \
         './temp-file';\nexport const services = { flags: { create: () => new FlagsService() } \
         };\nexport class FlagsService {\n  constructor(\n    private readonly flagsFile = \
         tempFile({\n      prefix: 'flags-core-',\n      suffix: '.json',\n    }),\n  ) {}\n  \
         async start() {\n    await writeFile(this.flagsFile, '{}');\n  }\n  async getTestEnv(): \
         Promise<Record<string, string>> {\n    return {\n      FLAGS_TEST_DATAFILE: \
         this.flagsFile,\n      FLAGS_SDK_KEY: 'fake',\n    };\n  }\n}\n",
    );
    write(
        r,
        "server/flags.test.ts",
        "import os from 'node:os';\nimport path from 'node:path';\nimport { FlagsService } from \
         './flags';\ntest('flags', async () => {\n  const datafilePath = path.join(os.tmpdir(), \
         'flags.json');\n  const service = new FlagsService(datafilePath);\n  const env = await \
         service.getTestEnv();\n  expect(env.FLAGS_TEST_DATAFILE).toBe(datafilePath);\n});\n",
    );
    // The temporary directory is itself a variable; a test points it at a directory
    // it makes.
    write(
        r,
        "server/tmp.test.ts",
        "import fs from 'node:fs';\nimport os from 'node:os';\nimport path from \
         'node:path';\nprocess.env.TMP_FOLDER = fs.mkdtempSync(path.join(os.tmpdir(), \
         't-'));\ntest('t', () => {});\n",
    );
    let client = "src/client.test.js".to_owned();
    // The file's own facts cannot evaluate the assignment (the constructor's
    // argument is bound by callers in other files), so the reader keeps its
    // record: `--strict` counts it. A cross-file evaluator behind
    // `Graph::env_patch_with` closes it
    // (`crates/turborepo-trace-store/tests/env_registry.rs`).
    assert!(strict(r, &["unrelated.txt"]).contains(&client));
    // The assigning file is an input of the reader either way.
    assert!(affected(r, &["server/flags.ts"]).contains(&client));
}

/// A variable as the directory of a path: whatever sets it, the file read ends
/// with the literal part, so every such file in the repository is a dependency.
#[test]
fn variable_directory_with_a_literal_path() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/seed.js",
        "const fs = require('node:fs');\nconst path = require('node:path');\nmodule.exports = () \
         => fs.readFileSync(path.join(process.env.DATA_DIR, 'seed', 'a.json'), 'utf8');\n",
    );
    write(
        r,
        "src/seed.test.js",
        "require('./seed');\ntest('s', () => {});\n",
    );
    write(r, "data/seed/a.json", "{}\n");
    write(r, "data/seed/b.json", "{}\n");
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    assert_eq!(affected(r, &["data/seed/a.json"]), ["src/seed.test.js"]);
    assert_eq!(affected(r, &["data/seed/b.json"]), none());
    // A new file that the path can name.
    write(r, "other/seed/a.json", "{}\n");
    assert_eq!(affected(r, &["other/seed/a.json"]), ["src/seed.test.js"]);
    // An assignment nobody can evaluate changes nothing: whatever the directory is,
    // the literal part bounds the path. The assigning file is an input of the
    // reader.
    write(
        r,
        "scripts/run.js",
        "process.env.DATA_DIR = require('some-package').dataDir();\n",
    );
    assert!(!strict(r, &["unrelated.txt"]).contains(&"src/seed.test.js".to_owned()));
    assert_eq!(affected(r, &["scripts/run.js"]), ["src/seed.test.js"]);
}

/// A relative value is relative to the working directory of whoever runs the
/// reader.
#[test]
fn relative_value() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    write(r, "services/a/dev/local-flags.json", "{}\n");
    write(
        r,
        "services/a/dev/start.js",
        "require('node:child_process').spawn('node', ['server.js'], {\n  env: { ...process.env, \
         FLAGS_TEST_DATAFILE: 'dev/local-flags.json' },\n});\n",
    );
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    assert_eq!(
        affected(r, &["services/a/dev/local-flags.json"]),
        ["src/client.test.js"]
    );
}

/// A module load from the environment: with every assignment evaluated it loads
/// those modules.
#[test]
fn module_load_from_the_environment() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/plugins.js",
        "module.exports = () => require(process.env.WIDGET_PLUGIN);\n",
    );
    write(
        r,
        "src/plugins.test.js",
        "const path = require('node:path');\nprocess.env.WIDGET_PLUGIN = path.join(__dirname, \
         '..', 'plugins', 'a.js');\nrequire('./plugins');\ntest('p', () => {});\n",
    );
    write(r, "plugins/a.js", "module.exports = require('./shared');\n");
    write(r, "plugins/shared.js", "module.exports = 1;\n");
    write(r, "plugins/b.js", "module.exports = 2;\n");
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    assert_eq!(affected(r, &["plugins/a.js"]), ["src/plugins.test.js"]);
    // The loaded module's own imports are dependencies too.
    assert_eq!(affected(r, &["plugins/shared.js"]), ["src/plugins.test.js"]);
    assert_eq!(affected(r, &["plugins/b.js"]), none());
}

/// What something outside the repository's code sets a variable to is declared
/// in the hints file.
#[test]
fn declared_value() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    write(r, "ci/flags.json", "{}\n");
    assert_eq!(affected(r, &["ci/flags.json"]), none());
    write(
        r,
        "hints.toml",
        "[[env]]\nname = \"FLAGS_TEST_DATAFILE\"\nvalues = [\"ci/flags.json\"]\n",
    );
    assert_eq!(affected(r, &["ci/flags.json"]), ["src/client.test.js"]);
    assert_eq!(strict(r, &["unrelated.txt"]), none());
}

/// `TURBO_TRACE_DEBUG_ENV=0` at query time leaves the records as the extractor
/// wrote them.
#[test]
fn switch_off() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    assert_eq!(strict(r, &["unrelated.txt"]), none());
    let o = Command::new(support::harness())
        // The hints file of these tests (`turbo trace` reads none).
        .env("TURBO_TRACE_DEBUG_HINTS", r.join("hints.toml"))
        .args([
            "affected",
            "--tests",
            "jest",
            "--strict",
            "--changed",
            "unrelated.txt",
        ])
        .current_dir(r)
        .env("TURBO_TRACE_DEBUG_ENV", "0")
        .output()
        .expect("run the harness");
    assert!(o.status.success());
    assert_eq!(
        String::from_utf8(o.stdout).unwrap().trim(),
        "src/client.test.js"
    );
}

/// A module load whose variable the repository never assigns names code from
/// outside: nothing bounds it, so it stays open until it is declared.
#[test]
fn module_load_nobody_assigns_stays_open() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "src/plugins.js",
        "module.exports = () => require(process.env.WIDGET_PLUGIN);\n",
    );
    write(
        r,
        "src/plugins.test.js",
        "require('./plugins');\ntest('p', () => {});\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), ["src/plugins.test.js"]);
    write(
        r,
        "hints.toml",
        "[[env]]\nname = \"WIDGET_PLUGIN\"\nvalues = []\n",
    );
    assert_eq!(affected(r, &["unrelated.txt"]), none());
}

/// The registry's patch is kept in the store between queries and recomputed
/// when the graph changes.
#[test]
fn patch_is_cached_by_the_manifest() {
    let d = repo();
    let r = d.path();
    write(r, "src/client.js", CLIENT);
    write(r, "src/client.test.js", CLIENT_TEST);
    write(r, "test/fixtures/flags.json", "{}\n");
    write(r, "test/fixtures/other.json", "{}\n");
    write(
        r,
        "test/setup.js",
        "const path = require('node:path');\nprocess.env.FLAGS_TEST_DATAFILE = \
         path.join(__dirname, 'fixtures', 'flags.json');\n",
    );
    let stats = |changed: &str| -> (String, Vec<String>) {
        let o = Command::new(support::harness())
            // The hints file of these tests (`turbo trace` reads none).
            .env("TURBO_TRACE_DEBUG_HINTS", r.join("hints.toml"))
            .args([
                "affected",
                "--tests",
                "jest",
                "--stats",
                "--changed",
                changed,
            ])
            .current_dir(r)
            .env_remove("TURBO_TRACE_DEBUG_ENV")
            .env_remove("TURBO_TRACE_DEBUG_DERIVED_CACHE")
            .output()
            .expect("run the harness");
        assert!(o.status.success());
        let err = String::from_utf8(o.stderr).unwrap();
        let line = err
            .lines()
            .find(|l| l.starts_with("env:"))
            .unwrap_or_default()
            .to_owned();
        let out = String::from_utf8(o.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        (line, out)
    };
    let (first, sel) = stats("test/fixtures/flags.json");
    assert!(!first.contains("(cached)"), "{first}");
    assert_eq!(sel, ["src/client.test.js"]);
    let (second, sel) = stats("test/fixtures/flags.json");
    assert!(second.contains("(cached)"), "{second}");
    assert_eq!(sel, ["src/client.test.js"]);
    assert_eq!(first, second.replace(" (cached)", ""));
    // The assignment now names another fixture: the graph changed, the patch is
    // recomputed.
    write(
        r,
        "test/setup.js",
        "const path = require('node:path');\nprocess.env.FLAGS_TEST_DATAFILE = \
         path.join(__dirname, 'fixtures', 'other.json');\n",
    );
    let (third, sel) = stats("test/fixtures/other.json");
    assert!(!third.contains("(cached)"), "{third}");
    assert_eq!(sel, ["src/client.test.js"]);
    let (_, sel) = stats("test/fixtures/flags.json");
    assert!(sel.is_empty(), "{sel:?}");
}

/// A package script sets the variable on its command line (`X=path cmd`): the
/// path is relative to the package, and the module the reader loads from it is
/// a dependency.
#[test]
fn assignment_on_a_package_script_command_line() {
    let d = repo();
    let r = d.path();
    write(
        r,
        "pnpm-workspace.yaml",
        "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
    );
    write(
        r,
        "apps/conf/package.json",
        r#"{"name":"conf","version":"1.0.0","scripts":{"seed":"SEED_CONFIG_PATH=../../packages/events/seed.config.ts tsx ../../packages/seeder/src/seed.ts","other":"OTHER_MODE=1 SEED_CONFIG_PATH=../../packages/events/seed.config.ts node x.js"}}"#,
    );
    write(
        r,
        "packages/seeder/package.json",
        r#"{"name":"seeder","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/events/package.json",
        r#"{"name":"events","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/seeder/src/seed.ts",
        "import path from 'node:path';\nconst configPath = process.env.SEED_CONFIG_PATH\n  ? \
         path.resolve(process.cwd(), process.env.SEED_CONFIG_PATH)\n  : './seed.config';\nexport \
         const config = import(configPath).then((mod) => mod.default);\n",
    );
    write(
        r,
        "packages/seeder/src/seed.config.ts",
        "export default {};\n",
    );
    write(
        r,
        "packages/seeder/src/seed.test.ts",
        "import './seed';\ntest('s', () => {});\n",
    );
    write(
        r,
        "packages/events/seed.config.ts",
        "import { collections } from './collections';\nexport default { collections };\n",
    );
    write(
        r,
        "packages/events/collections.ts",
        "export const collections = [];\n",
    );
    let test = ["packages/seeder/src/seed.test.ts"];
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    // The module the script names, and what it imports.
    assert_eq!(affected(r, &["packages/events/seed.config.ts"]), test);
    assert_eq!(affected(r, &["packages/events/collections.ts"]), test);
}

/// A launcher passes the variable to the process it starts, as a literal path
/// relative to its working directory, which is not known: the module is found
/// in the repository directory that holds it, and a module created at the same
/// path elsewhere counts too.
#[test]
fn module_path_relative_to_an_unknown_working_directory() {
    let d = repo();
    let r = d.path();
    write(r, "pnpm-workspace.yaml", "packages:\n  - 'packages/*'\n");
    write(
        r,
        "packages/seeder/package.json",
        r#"{"name":"seeder","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/events/package.json",
        r#"{"name":"events","version":"1.0.0"}"#,
    );
    write(
        r,
        "packages/seeder/scripts/launch.mjs",
        "import { spawnSync } from 'node:child_process';\nimport { resolve } from \
         'node:path';\nfunction findConfig() {\n  if (process.env.SEED_CONFIG_PATH) {\n    return \
         resolve(process.cwd(), process.env.SEED_CONFIG_PATH);\n  }\n  return \
         resolve(process.cwd(), '../../packages/events/seed.config.ts');\n}\nconst configPath = \
         findConfig();\nspawnSync('tsx', ['src/seed.ts'], {\n  env: { ...process.env, \
         SEED_CONFIG_PATH: configPath },\n});\n",
    );
    write(
        r,
        "packages/seeder/src/seed.ts",
        "import path from 'node:path';\nconst configPath = process.env.SEED_CONFIG_PATH\n  ? \
         path.resolve(process.cwd(), process.env.SEED_CONFIG_PATH)\n  : './seed.config';\nexport \
         const config = import(configPath).then((mod) => mod.default);\n",
    );
    write(
        r,
        "packages/seeder/src/seed.config.ts",
        "export default {};\n",
    );
    write(
        r,
        "packages/seeder/src/seed.test.ts",
        "import './seed';\ntest('s', () => {});\n",
    );
    write(
        r,
        "packages/events/seed.config.ts",
        "import { collections } from './collections';\nexport default { collections };\n",
    );
    write(
        r,
        "packages/events/collections.ts",
        "export const collections = [];\n",
    );
    let test = ["packages/seeder/src/seed.test.ts"];
    // The record is closed: an unrelated change selects nothing.
    assert_eq!(affected(r, &["unrelated.txt"]), none());
    assert_eq!(affected(r, &["packages/events/seed.config.ts"]), test);
    assert_eq!(affected(r, &["packages/events/collections.ts"]), test);
    // The launcher that assigns the variable is an input of the reader.
    assert_eq!(affected(r, &["packages/seeder/scripts/launch.mjs"]), test);
    // The same path below another directory: a module the launcher could name from
    // there.
    write(
        r,
        "examples/packages/events/seed.config.ts",
        "export default {};\n",
    );
    assert_eq!(
        affected(r, &["examples/packages/events/seed.config.ts"]),
        test
    );
}
