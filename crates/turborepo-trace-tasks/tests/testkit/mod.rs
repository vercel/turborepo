//! Registry-wide test kit for command recognizers, in the spirit
//! of `turborepo-trace-testkit` for language plugins.
//!
//! - [`SyntheticRepo`]: a small throwaway repository (JS/TS workspace, Prisma,
//!   GraphQL, protobuf, sqlc, OpenAPI, Python, Go, Cargo, Make, Docker) written
//!   to a temporary directory and removed on drop.
//! - [`Gen`]: a seeded generator of commands for any recognizer: its declared
//!   programs with arbitrary flags, subcommands, paths, unicode, control
//!   characters, empty and huge words.
//! - [`check_case`]: runs one command through one recognizer (directly) and
//!   through the whole [`registry`], and checks the invariants every recognizer
//!   must keep (no panic, bounded time, deterministic, absolute in-repo paths,
//!   `Full` never with an unknown-command uncertainty, only declared programs
//!   claimed).
//! - [`fixture_text`] and [`recognizer_fixture!`]: per-tool fixtures (commands
//!   → recognized `TaskSpec`, snapshotted with insta), with the same invariants
//!   checked on every command.
//!
//! Everything iterates [`registry`]`().recognizers()`, so a newly registered
//! recognizer (hand written or spec based) is covered without editing this
//! file.
//!
//! `TURBO_TRACE_DEBUG_TESTKIT_SEED` changes the seed,
//! `TURBO_TRACE_DEBUG_TESTKIT_ITERS` the number of random commands per declared
//! program, and `TURBO_TRACE_DEBUG_TESTKIT_MAX_MS` the per-call time budget.
//! Every failure message includes the offending argv (escaped) so it can become
//! a unit test.
#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_tasks::{
    registry::{CommandReport, Recognition, RecognizeCx, Recognizer, WRAPPERS, registry},
    shell,
    spec::{Command, Redirect, TaskSpec, Word},
};

// ---- Configuration
// ---------------------------------------------------------------------------

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

/// The base seed (`TURBO_TRACE_DEBUG_TESTKIT_SEED`, default fixed).
pub fn seed() -> u64 {
    env_u64("TURBO_TRACE_DEBUG_TESTKIT_SEED").unwrap_or(0x7a5c_2026_1103_0001)
}

/// Random commands per declared program (`TURBO_TRACE_DEBUG_TESTKIT_ITERS`,
/// default `default`).
pub fn iters(default: usize) -> usize {
    env_u64("TURBO_TRACE_DEBUG_TESTKIT_ITERS").map_or(default, |n| n as usize)
}

/// Time budget for one recognizer call (`TURBO_TRACE_DEBUG_TESTKIT_MAX_MS`,
/// default 2 s: generous for debug builds and noisy CI machines; a hang or a
/// quadratic blow-up on a huge argument still trips it).
pub fn max_call_time() -> Duration {
    Duration::from_millis(env_u64("TURBO_TRACE_DEBUG_TESTKIT_MAX_MS").unwrap_or(2000))
}

// ---- RNG ---------------------------------------------------------------------------------------

/// xorshift64* (deterministic, not cryptographic).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator for `seed` (zero is replaced by a constant).
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
    /// Next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A value in `0..n` (0 for `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
    /// `true` with probability `1/n`.
    pub fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }
    /// A random element.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// FNV-1a, for per-recognizer seeds.
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ---- Synthetic repository
// ----------------------------------------------------------------------

/// Files of the synthetic repository (path, contents).
const REPO_FILES: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{"name":"synthetic","private":true,"workspaces":["apps/*","packages/*"],
"scripts":{"build":"tsc -p .","test":"jest","gen":"prisma generate","lint":"eslint ."},
"devDependencies":{"typescript":"5.4.0","jest":"29.0.0","prisma":"5.0.0","vite":"5.0.0","first-party":"workspace:*"}}"#,
    ),
    (
        "pnpm-workspace.yaml",
        "packages:\n  - apps/*\n  - packages/*\n",
    ),
    (
        "turbo.json",
        r#"{"tasks":{"build":{"dependsOn":["^build"],"outputs":["dist/**"]}}}"#,
    ),
    (
        "tsconfig.json",
        r#"{"compilerOptions":{"outDir":"dist","rootDir":"src"},"include":["src/**/*"]}"#,
    ),
    (
        "tsconfig.build.json",
        r#"{"extends":"./tsconfig.json","exclude":["**/*.test.ts"]}"#,
    ),
    (
        "vite.config.ts",
        "export default { build: { outDir: 'dist' } };\n",
    ),
    (
        "vitest.config.ts",
        "export default { test: { include: ['src/**/*.test.ts'] } };\n",
    ),
    (
        "jest.config.js",
        "module.exports = { testMatch: ['**/*.test.ts'], setupFiles: ['./jest.setup.js'] };\n",
    ),
    ("jest.setup.js", ""),
    (
        "webpack.config.js",
        "module.exports = { entry: './src/index.ts', output: { path: __dirname + '/dist' } };\n",
    ),
    (
        "rollup.config.mjs",
        "export default { input: 'src/index.ts', output: { dir: 'dist' } };\n",
    ),
    (
        "tsup.config.ts",
        "export default { entry: ['src/index.ts'], outDir: 'dist' };\n",
    ),
    (
        "prisma/schema.prisma",
        "generator client {\n  provider = \"prisma-client-js\"\n  output = \
         \"../src/generated/prisma\"\n}\ndatasource db {\n  provider = \"postgresql\"\n  url = \
         env(\"DATABASE_URL\")\n}\nmodel User {\n  id Int @id\n}\n",
    ),
    (
        "codegen.yml",
        "schema: schema.graphql\ndocuments: 'src/**/*.graphql'\ngenerates:\n  \
         src/generated/graphql.ts:\n    plugins: [typescript]\n",
    ),
    ("schema.graphql", "type Query { a: Int }\n"),
    ("buf.yaml", "version: v2\nmodules:\n  - path: proto\n"),
    (
        "buf.gen.yaml",
        "version: v2\nplugins:\n  - local: protoc-gen-go\n    out: gen/go\n",
    ),
    (
        "proto/a.proto",
        "syntax = \"proto3\";\npackage a;\nmessage A { int32 x = 1; }\n",
    ),
    (
        "sqlc.yaml",
        "version: \"2\"\nsql:\n  - schema: db/schema.sql\n    queries: db/query.sql\n    engine: \
         postgresql\n    gen:\n      go:\n        package: db\n        out: internal/db\n",
    ),
    ("db/schema.sql", "create table a (id int);\n"),
    ("db/query.sql", "-- name: GetA :one\nselect * from a;\n"),
    (
        "openapi.yaml",
        "openapi: 3.0.0\ninfo: {title: t, version: '1'}\npaths: {}\n",
    ),
    ("src/index.ts", "import './a';\nexport const x = 1;\n"),
    ("src/a.ts", "export const a = 1;\n"),
    ("src/a.test.ts", "import { a } from './a';\n"),
    ("src/main.rs", "fn main() {}\n"),
    (
        "scripts/gen.mjs",
        "import fs from 'node:fs';\nfs.writeFileSync('out.txt', '');\n",
    ),
    ("scripts/gen.py", "open('out.txt', 'w').write('')\n"),
    ("scripts/build.sh", "#!/bin/sh\necho build\n"),
    (
        "pyproject.toml",
        "[project]\nname = \"synthetic\"\nversion = \"0.1.0\"\n[project.scripts]\nsynth = \
         \"synthetic.cli:main\"\n[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n",
    ),
    ("synthetic/__init__.py", ""),
    ("synthetic/cli.py", "def main():\n    pass\n"),
    ("tests/test_a.py", "def test_a():\n    pass\n"),
    ("requirements.txt", "pytest\n"),
    ("go.mod", "module example.com/synthetic\n\ngo 1.22\n"),
    ("main.go", "package main\n\nfunc main() {}\n"),
    (
        "gen.go",
        "package main\n\n//go:generate stringer -type=Color\ntype Color int\n",
    ),
    ("cmd/x/main.go", "package main\n\nfunc main() {}\n"),
    (
        "Cargo.toml",
        "[package]\nname = \"synthetic\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    ),
    (
        "Makefile",
        "all: dist/app\n\ndist/app: main.go\n\tgo build -o dist/app .\n",
    ),
    (
        "Dockerfile",
        "FROM node:20 AS build\nCOPY package.json .\nCOPY src ./src\nRUN npm run build\n",
    ),
    (".dockerignore", "node_modules\n"),
    ("docker-compose.yml", "services:\n  web:\n    build: .\n"),
    ("justfile", "build:\n    tsc -p .\n"),
    (
        "Taskfile.yml",
        "version: '3'\ntasks:\n  build:\n    cmds:\n      - tsc -p .\n",
    ),
    (".env", "A=1\n"),
    (
        "node_modules/typescript/package.json",
        r#"{"name":"typescript","version":"5.4.0","bin":{"tsc":"bin/tsc"}}"#,
    ),
    ("node_modules/typescript/bin/tsc", "#!/usr/bin/env node\n"),
    ("node_modules/.bin/tsc", "#!/usr/bin/env node\n"),
    (
        "apps/web/package.json",
        r#"{"name":"web","bin":{"web-cli":"cli.js"},"scripts":{"build":"next build","dev":"vite"},"dependencies":{"first-party":"workspace:*"}}"#,
    ),
    (
        "apps/web/cli.js",
        "#!/usr/bin/env node\nrequire('./lib');\n",
    ),
    ("apps/web/lib.js", ""),
    ("apps/web/next.config.js", "module.exports = {};\n"),
    (
        "apps/web/tsconfig.json",
        r#"{"extends":"../../tsconfig.json"}"#,
    ),
    ("apps/web/src/page.tsx", "export default function P() {}\n"),
    (
        "packages/first-party/package.json",
        r#"{"name":"first-party","bin":{"fp":"bin/fp.js"}}"#,
    ),
    ("packages/first-party/bin/fp.js", "#!/usr/bin/env node\n"),
];

/// Config files written to the parent of the repository root (outside the
/// repo).
const DECOYS: &[(&str, &str)] = &[
    (
        "package.json",
        r#"{"name":"decoy","scripts":{"build":"x"}}"#,
    ),
    ("tsconfig.json", r#"{"compilerOptions":{"outDir":"decoy"}}"#),
    ("jest.config.js", "module.exports = {};\n"),
    ("vitest.config.ts", "export default {};\n"),
    ("vite.config.ts", "export default {};\n"),
    ("pyproject.toml", "[project]\nname = \"decoy\"\n"),
    ("pytest.ini", "[pytest]\n"),
    ("setup.cfg", "[metadata]\nname = decoy\n"),
    ("go.mod", "module decoy\n"),
    ("Cargo.toml", "[workspace]\n"),
    ("buf.gen.yaml", "version: v2\n"),
    ("codegen.yml", "schema: x.graphql\n"),
    ("sqlc.yaml", "version: \"2\"\n"),
    (
        "schema.prisma",
        "generator client {\n  provider = \"prisma-client-js\"\n}\n",
    ),
    (".env", "DECOY=1\n"),
    ("uv.lock", ""),
    ("poetry.lock", ""),
    (".babelrc", "{}"),
    (".eslintrc.json", "{}"),
    (".prettierrc", "{}"),
    ("node_modules/.bin/decoy-bin", "#!/bin/sh\n"),
    (
        "node_modules/decoy-bin/package.json",
        r#"{"name":"decoy-bin","bin":"x.js"}"#,
    ),
];

/// Package roots of the synthetic repository, repo-relative.
pub const PACKAGES: &[&str] = &["", "apps/web", "packages/first-party"];

/// Command working directories, repo-relative (each inside one of
/// [`PACKAGES`]).
pub const CWDS: &[(&str, &str)] = &[
    ("", ""),
    ("src", ""),
    ("prisma", ""),
    ("apps/web", "apps/web"),
    ("apps/web/src", "apps/web"),
    ("packages/first-party", "packages/first-party"),
];

/// A throwaway repository on disk, removed on drop.
pub struct SyntheticRepo {
    /// The repository root (canonical, absolute).
    pub root: PathBuf,
    tmp: PathBuf,
    /// Script names of the root `package.json`.
    pub scripts: Vec<String>,
}

impl SyntheticRepo {
    /// Writes the repository to a fresh temporary directory.
    pub fn new() -> SyntheticRepo {
        static N: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::temp_dir().join(format!(
            "turbotrace-tasks-testkit-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        // Decoys above the repo root: a recognizer that searches upwards without
        // stopping at the root picks them up, and the inside-repo check reports
        // it.
        for (rel, contents) in DECOYS {
            let p = tmp.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, contents).unwrap();
        }
        let repo = tmp.join("repo");
        for (rel, contents) in REPO_FILES {
            let p = repo.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, contents).unwrap();
        }
        // Canonical, so paths compare equal to what recognizers build (macOS /var →
        // /private/var).
        let root = repo.canonicalize().unwrap();
        SyntheticRepo {
            root,
            tmp,
            scripts: ["build", "test", "gen", "lint"].map(String::from).to_vec(),
        }
    }
}

impl Default for SyntheticRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SyntheticRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

// ---- Command generation
// ------------------------------------------------------------------------

/// Option-looking words recognizers commonly parse.
pub const FLAGS: &[&str] = &[
    "-p",
    "--project",
    "-c",
    "--config",
    "-o",
    "--out",
    "--outDir",
    "--out-dir",
    "--outdir",
    "--outfile",
    "-w",
    "--watch",
    "-r",
    "-C",
    "-f",
    "--file",
    "-I",
    "--proto_path",
    "-m",
    "--",
    "-",
    "--schema",
    "--input",
    "-i",
    "--output",
    "-d",
    "--dir",
    "--filter",
    "-F",
    "--env-file",
    "-e",
    "--noEmit",
    "-b",
    "--build",
    "--mode",
    "--target",
    "--format",
    "--entry",
    "--root",
    "--cwd",
    "--prefix",
    "-x",
    "-v",
    "--help",
    "--version",
    "--require",
    "--import",
    "--loader",
    "-t",
    "--coverage",
    "--bundle",
    "--generator",
    "--plugin",
    "--go_out",
    "--ts_out",
    "--python_out",
    "-g",
    "--template",
    "--package",
    "-source",
    "-destination",
    "-type",
    "-output",
    "--workspace",
    "-s",
    "--silent",
    "--recursive",
    "--parallel",
    "-k",
    "--names",
    "--tags",
    "-run",
    "--manifest-path",
    "-j",
    "--selectProjects",
    "--rootDir",
    "--dotenv",
];

/// Subcommand-looking words.
pub const SUBCOMMANDS: &[&str] = &[
    "run", "build", "generate", "test", "exec", "tool", "dev", "start", "serve", "preview",
    "watch", "migrate", "push", "pull", "install", "x", "dlx", "lint", "format", "check",
    "compile", "bundle", "codegen", "vet", "fmt", "mod", "init", "db", "deploy", "clean", "all",
    "dist", "up", "compose", "-m", "unittest", "pip", "pytest", "script", "cp", "mv", "ln",
    "touch", "mkdir", "rm", "gen", "nextest", "bench", "xtask", "dev", "e2e",
];

/// Relative paths (inside the repository; none climbs out).
pub const PATHS: &[&str] = &[
    ".",
    "src",
    "src/index.ts",
    "src/a.test.ts",
    "tsconfig.json",
    "tsconfig.build.json",
    "prisma/schema.prisma",
    "codegen.yml",
    "proto",
    "proto/a.proto",
    "buf.gen.yaml",
    "sqlc.yaml",
    "openapi.yaml",
    "scripts/gen.mjs",
    "scripts/gen.py",
    "scripts/build.sh",
    "dist",
    "build",
    "out",
    "generated/types.ts",
    "apps/web",
    "apps/web/cli.js",
    "main.go",
    "./cmd/x",
    "./...",
    "cmd/x/main.go",
    "tests",
    "tests/test_a.py",
    "Makefile",
    "Dockerfile",
    "does/not/exist.ts",
    "src/**/*.ts",
    "*.json",
    "vite.config.ts",
    "jest.config.js",
    "package.json",
    ".env",
    "node_modules/.bin/tsc",
    "./src/index.ts",
    "src/",
    "db/query.sql",
    "synthetic.cli",
    "synthetic",
    "main.rs",
    "web",
    "first-party",
    "@scope/pkg",
];

/// Paths that leave the repository (explicitly written by the command).
pub const OUTSIDE_PATHS: &[&str] = &[
    "..",
    "../x",
    "../../etc/passwd",
    "/etc/hosts",
    "/tmp/turbotrace-testkit-outside",
    "~/x",
    "-I..",
    "-I../proto",
    "--config=/abs/c.json",
    "--out-dir=../dist",
    "./../../..",
    "src/../../x",
];

/// Single words that broke parsers before, or are likely to: empty and blank
/// words, vertical tab, form feed, NBSP, U+2028/U+2029 line and paragraph
/// separators, BOM, NUL, ANSI escapes, combining marks, bidi overrides, astral
/// characters, shell metacharacters.
pub const NASTY: &[&str] = &[
    "",
    " ",
    "\u{0B}",
    "\u{0C}",
    "\u{A0}",
    "\u{2028}",
    "\u{2029}",
    "\u{FEFF}",
    "\0",
    "\t",
    "\n",
    "\r\n",
    "\r",
    "\u{1B}[31m",
    "\u{7F}",
    "\u{85}",
    "é",
    "e\u{301}",
    "\u{202E}gnp.exe",
    "日本語",
    "🦀",
    "--=",
    "=",
    "-",
    "--",
    "---",
    "-=-",
    ",",
    ":",
    ";",
    "'",
    "\"",
    "\\",
    "$",
    "${",
    "$(x)",
    "`",
    "*",
    "**",
    "?",
    "[",
    "{a,b}",
    "%",
    "#",
    "@",
    "-\u{A0}p",
    "--config=\u{2028}",
    "a\u{0B}b",
    "tsconfig.json\u{0C}",
    "src\u{A0}index.ts",
    "-p\u{2028}",
    "\u{0B}-c",
    "--out=\u{0C}",
    "\u{2028}\u{2028}",
    "-\u{0B}",
    "=\u{A0}=",
    "src/\u{2028}.ts",
];

/// The regression characters every recognizer must survive anywhere in a word.
pub const REGRESSION_CHARS: &[char] = &['\u{0B}', '\u{0C}', '\u{A0}', '\u{2028}'];

const CHAR_POOL: &[char] = &[
    'a',
    'b',
    'z',
    'A',
    'Z',
    '0',
    '9',
    '-',
    '_',
    '.',
    '=',
    ',',
    ':',
    '@',
    '+',
    '#',
    '%',
    '*',
    '?',
    '[',
    ']',
    '{',
    '}',
    ' ',
    '\t',
    '\n',
    '\r',
    '\0',
    '\u{1}',
    '\u{1B}',
    '\u{7F}',
    '\u{0B}',
    '\u{0C}',
    '\u{A0}',
    '\u{2028}',
    '\u{2029}',
    '\u{FEFF}',
    '\u{200B}',
    '\u{301}',
    '\u{202E}',
    'é',
    'ß',
    'λ',
    '日',
    '🦀',
    '\u{10FFFF}',
    '\'',
    '"',
    '\\',
    '$',
    '`',
    '!',
    '&',
    '|',
    ';',
    '<',
    '>',
    '(',
    ')',
];

/// Whether `w` could name a path outside the repository: a `..` component, or
/// an absolute or home-relative path, also after a flag prefix (`-I..`,
/// `--out=/x`).
fn may_escape(w: &str) -> bool {
    w.contains("..")
        || w.contains('~')
        || w.starts_with('/')
        || ["=/", ",/", ":/", "-/"].iter().any(|s| w.contains(s))
}

/// Makes a random word safe for the in-repo phase.
fn confine(w: String) -> String {
    let mut w = w.replace('~', "_");
    while w.contains("..") {
        w = w.replace("..", ".");
    }
    while w.starts_with('/') {
        w.remove(0);
    }
    if may_escape(&w) { "x".to_string() } else { w }
}

/// How words are generated.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every path-like word stays inside the repository: any emitted path
    /// outside the root is a bug (unless flagged with an `outside_base`
    /// uncertainty).
    Confined,
    /// Words may name paths outside the repository; paths must still be
    /// absolute and normalized.
    Outside,
    /// Every word carries at least one of [`REGRESSION_CHARS`].
    Regression,
    /// One huge word or very many words.
    Huge,
}

/// One generated command with its context.
#[derive(Clone, Debug)]
pub struct Case {
    /// The command.
    pub cmd: Command,
    /// Package root (absolute).
    pub package_root: PathBuf,
    /// Generation mode.
    pub mode: Mode,
}

impl Case {
    /// The argv, escaped, for failure messages.
    pub fn show(&self) -> String {
        let argv: Vec<String> = self
            .cmd
            .argv
            .iter()
            .map(|w| {
                let mut s = if w.text.len() > 80 {
                    format!(
                        "{:?}…({} bytes)",
                        &w.text[..w.text.floor_char_boundary(40)],
                        w.text.len()
                    )
                } else {
                    format!("{:?}", w.text)
                };
                if w.dynamic {
                    s.push_str("(dyn)");
                }
                if w.glob {
                    s.push_str("(glob)");
                }
                s
            })
            .collect();
        let argv = if argv.len() > 12 {
            format!("{} … ({} words)", argv[..12].join(" "), argv.len())
        } else {
            argv.join(" ")
        };
        let extra = if self.cmd.assigns.is_empty() && self.cmd.redirects.is_empty() {
            String::new()
        } else {
            format!(
                " assigns={:?} redirects={:?}",
                self.cmd
                    .assigns
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.text))
                    .collect::<Vec<_>>(),
                self.cmd
                    .redirects
                    .iter()
                    .map(|r| format!("{}{}", r.op, r.target.text))
                    .collect::<Vec<_>>()
            )
        };
        format!(
            "[{argv}]{extra} cwd={} mode={:?}",
            self.cmd.cwd.display(),
            self.mode
        )
    }
}

/// Seeded command generator over a [`SyntheticRepo`].
pub struct Gen<'a> {
    /// The repository.
    pub repo: &'a SyntheticRepo,
    /// The RNG.
    pub rng: Rng,
    programs: Vec<&'static str>,
}

impl<'a> Gen<'a> {
    /// A generator seeded with `seed`.
    pub fn new(repo: &'a SyntheticRepo, seed: u64) -> Gen<'a> {
        let mut programs: Vec<&'static str> = registry()
            .recognizers()
            .iter()
            .flat_map(|r| r.programs().iter().copied())
            .collect();
        programs.extend(WRAPPERS.iter().copied());
        programs.extend([
            "pnpm",
            "npm",
            "yarn",
            "bun",
            "turbo",
            "concurrently",
            "run-s",
            "sh",
            "bash",
        ]);
        programs.sort_unstable();
        programs.dedup();
        Gen {
            repo,
            rng: Rng::new(seed),
            programs,
        }
    }

    fn random_string(&mut self, max: usize) -> String {
        let n = self.rng.below(max + 1);
        (0..n).map(|_| *self.rng.pick(CHAR_POOL)).collect()
    }

    /// One argument word's text.
    pub fn word_text(&mut self, mode: Mode) -> String {
        let w = match self.rng.below(16) {
            0..=2 => (*self.rng.pick(FLAGS)).to_string(),
            3 => format!("{}={}", self.rng.pick(FLAGS), self.rng.pick(PATHS)),
            4 => format!(
                "{}{}",
                self.rng.pick(&["-I", "-o", "-c", "-p", "-r", "-e"]),
                self.rng.pick(PATHS)
            ),
            5 | 6 => (*self.rng.pick(SUBCOMMANDS)).to_string(),
            7..=9 => (*self.rng.pick(PATHS)).to_string(),
            10 => (*self.rng.pick(NASTY)).to_string(),
            11 => self.random_string(12),
            12 => (*self.rng.pick(&self.programs)).to_string(),
            13 => format!("{},{}", self.rng.pick(PATHS), self.rng.pick(PATHS)),
            14 => format!("{}={}", self.random_string(4), self.random_string(6)),
            _ => self.rng.below(100_000).to_string(),
        };
        match mode {
            Mode::Confined | Mode::Huge => {
                if PATHS.contains(&w.as_str()) {
                    w
                } else {
                    confine(w)
                }
            }
            Mode::Outside => {
                if self.rng.one_in(3) {
                    (*self.rng.pick(OUTSIDE_PATHS)).to_string()
                } else {
                    w
                }
            }
            Mode::Regression => {
                let w = confine(w);
                let c = *self.rng.pick(REGRESSION_CHARS);
                let at = self.rng.below(w.chars().count() + 1);
                let mut s: String = w.chars().take(at).collect();
                s.push(c);
                s.extend(w.chars().skip(at));
                s
            }
        }
    }

    fn word(&mut self, mode: Mode) -> Word {
        let text = self.word_text(mode);
        Word {
            glob: text.contains(['*', '?', '[']) && self.rng.one_in(2),
            dynamic: self.rng.one_in(12),
            text,
        }
    }

    /// A random command whose program is `program`.
    pub fn case(&mut self, program: &str, mode: Mode) -> Case {
        let (cwd, pkg) = *self.rng.pick(CWDS);
        let cwd = self.repo.root.join(cwd);
        let package_root = self.repo.root.join(pkg);
        let p0 = match self.rng.below(10) {
            0 => format!("node_modules/.bin/{program}"),
            1 => format!("./node_modules/.bin/{program}"),
            _ => program.to_string(),
        };
        let mut argv = vec![Word::lit(p0)];
        match mode {
            Mode::Huge => {
                if self.rng.one_in(2) {
                    // One huge argument.
                    let unit = self.word_text(Mode::Confined);
                    let unit = if unit.is_empty() {
                        "a".to_string()
                    } else {
                        unit
                    };
                    let mut big = String::with_capacity(256 * 1024);
                    while big.len() < 256 * 1024 {
                        big.push_str(&unit);
                    }
                    if self.rng.one_in(2) {
                        argv.push(Word::lit(self.rng.pick(SUBCOMMANDS).to_string()));
                    }
                    argv.push(Word::lit(big));
                } else {
                    // Very many arguments.
                    for _ in 0..3000 {
                        argv.push(self.word(Mode::Confined));
                    }
                }
            }
            _ => {
                // Often a subcommand first: many recognizers dispatch on it.
                if self.rng.one_in(2) {
                    argv.push(Word::lit(self.rng.pick(SUBCOMMANDS).to_string()));
                }
                let n = self.rng.below(9);
                for _ in 0..n {
                    argv.push(self.word(mode));
                }
            }
        }
        let mut assigns = Vec::new();
        if self.rng.one_in(4) {
            let k = *self.rng.pick(&[
                "NODE_ENV",
                "GOFLAGS",
                "CI",
                "TS_NODE_PROJECT",
                "DEBUG",
                "PRISMA_SCHEMA",
                "X",
            ]);
            let v = self.word_text(if mode == Mode::Outside {
                Mode::Outside
            } else {
                Mode::Confined
            });
            assigns.push((k.to_string(), Word::lit(v)));
        }
        let mut redirects = Vec::new();
        if self.rng.one_in(5) {
            let (op, target) = match self.rng.below(4) {
                0 => ("<", (*self.rng.pick(PATHS)).to_string()),
                1 => (">", "out/log.txt".to_string()),
                2 => (">>", "dist/x.txt".to_string()),
                _ => ("2>", "&1".to_string()),
            };
            redirects.push(Redirect {
                op: op.to_string(),
                target: Word::lit(target),
            });
        }
        let text = argv
            .iter()
            .map(|w| w.text.chars().take(64).collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        Case {
            cmd: Command {
                argv,
                assigns,
                redirects,
                cwd,
                cwd_computed: false,
                next: Default::default(),
                text,
            },
            package_root,
            mode,
        }
    }
}

// ---- Invariants
// --------------------------------------------------------------------------------

/// Which invariant failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prop {
    /// A call panicked.
    NoPanic,
    /// A call took longer than [`max_call_time`].
    BoundedTime,
    /// Two identical calls produced different specs or reports.
    Deterministic,
    /// A path is relative or not normalized (`.`/`..` components).
    AbsoluteNormalized,
    /// A path is outside the repository root without an `outside_base`
    /// uncertainty.
    InsideRepo,
    /// `Full` with an `unknown_command` uncertainty.
    FullNotUnknown,
    /// `matches` accepted a program the recognizer does not declare.
    DeclaredPrograms,
}

/// One invariant violation.
#[derive(Clone, Debug)]
pub struct Failure {
    /// The invariant.
    pub prop: Prop,
    /// The recognizer under test (or `registry`).
    pub recognizer: String,
    /// Details including the escaped input.
    pub message: String,
}

/// Every path of `spec` with what it is.
pub fn spec_paths(spec: &TaskSpec) -> Vec<(&'static str, &Path)> {
    let mut v: Vec<(&'static str, &Path)> = Vec::new();
    v.extend(spec.inputs.iter().map(|p| ("input", p.as_path())));
    v.extend(spec.configs.iter().map(|p| ("config", p.as_path())));
    v.extend(spec.outputs.iter().map(|o| ("output", o.path.as_path())));
    v.extend(spec.entries.iter().map(|e| ("entry", e.path.as_path())));
    v.extend(
        spec.memberships
            .iter()
            .map(|m| ("membership", m.dir.as_path())),
    );
    v
}

/// Whether `p` is absolute with no `.` or `..` component (lexically
/// normalized).
pub fn is_normalized_absolute(p: &Path) -> bool {
    if !p.is_absolute() {
        return false;
    }
    // `components()` hides inner `.`; check the text too.
    let s = p.to_string_lossy();
    !(s.contains("/./") || s.ends_with("/.") || s.contains("//"))
        && !p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}

/// Checks the path invariants of `spec` (normalized absolute paths; inside
/// `root` unless `confined` is false or the spec flags an `outside_base`
/// uncertainty).
pub fn check_paths(spec: &TaskSpec, root: &Path, confined: bool) -> Vec<(Prop, String)> {
    let mut out = Vec::new();
    let flagged = spec
        .uncertainties
        .iter()
        .any(|u| u.kind == UncertaintyKind::OutsideBase);
    for (what, p) in spec_paths(spec) {
        if !is_normalized_absolute(p) {
            out.push((
                Prop::AbsoluteNormalized,
                format!("{what} {p:?} is not absolute and normalized"),
            ));
        } else if confined && !flagged && !p.starts_with(root) {
            out.push((
                Prop::InsideRepo,
                format!("{what} {p:?} is outside the repo root and not flagged `outside_base`"),
            ));
        }
    }
    out
}

/// Checks `Recognition::Full` against the spec's uncertainties.
pub fn check_full(rec: Recognition, spec: &TaskSpec) -> Option<String> {
    (rec == Recognition::Full)
        .then(|| {
            spec.uncertainties
                .iter()
                .find(|u| u.kind == UncertaintyKind::UnknownCommand)
                .map(|u| format!("Full with unknown-command uncertainty `{}`", u.detail))
        })
        .flatten()
}

fn timed<T>(f: impl FnOnce() -> T) -> (Result<T, String>, Duration) {
    let t = Instant::now();
    let r = catch_unwind(AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic".into())
    });
    (r, t.elapsed())
}

/// Runs `cmd` through `r` directly (when it matches) and through the registry,
/// twice each, and returns every invariant violation.
pub fn check_case(r: &dyn Recognizer, case: &Case, repo: &SyntheticRepo) -> Vec<Failure> {
    let env = BTreeMap::new();
    let cx = RecognizeCx {
        repo_root: &repo.root,
        package_root: &case.package_root,
        env: &env,
        task_name: Some("build"),
        scripts: &repo.scripts,
    };
    let confined = matches!(case.mode, Mode::Confined | Mode::Regression | Mode::Huge);
    let budget = max_call_time();
    let mut fails = Vec::new();
    let mut fail = |prop: Prop, who: &str, msg: String| {
        fails.push(Failure {
            prop,
            recognizer: who.to_string(),
            message: format!("{msg}\n    input: {}", case.show()),
        });
    };

    // Which recognizers claim the command: only for programs they declare.
    let prog = case.cmd.program_name().unwrap_or_default();
    let under_test = std::iter::once(r).chain(
        registry()
            .recognizers()
            .iter()
            .copied()
            .filter(|o| o.id() != r.id()),
    );
    for other in under_test {
        if catch_unwind(AssertUnwindSafe(|| other.matches(&case.cmd))).unwrap_or(false)
            && !other.programs().contains(&prog)
        {
            fail(
                Prop::DeclaredPrograms,
                other.id(),
                format!(
                    "`{}` claims program {prog:?}, which is not in programs() {:?}",
                    other.id(),
                    other.programs()
                ),
            );
        }
    }

    // Direct.
    let direct = |r: &dyn Recognizer| {
        let mut spec = TaskSpec::default();
        let rec = r.recognize(&case.cmd, &cx, &mut spec);
        spec.normalize();
        (rec, spec)
    };
    let matched = catch_unwind(AssertUnwindSafe(|| r.matches(&case.cmd)));
    match matched {
        Err(_) => fail(Prop::NoPanic, r.id(), "matches() panicked".into()),
        Ok(false) => {}
        Ok(true) => {
            let (a, ta) = timed(|| direct(r));
            match a {
                Err(e) => fail(Prop::NoPanic, r.id(), format!("recognize() panicked: {e}")),
                Ok((rec, spec)) => {
                    if ta > budget {
                        fail(
                            Prop::BoundedTime,
                            r.id(),
                            format!("recognize() took {ta:?} (budget {budget:?})"),
                        );
                    }
                    for (p, m) in check_paths(&spec, &repo.root, confined) {
                        fail(p, r.id(), m);
                    }
                    if let Some(m) = check_full(rec, &spec) {
                        fail(Prop::FullNotUnknown, r.id(), m);
                    }
                    if let (Ok((rec2, spec2)), _) = timed(|| direct(r))
                        && (rec2 != rec || spec2 != spec)
                    {
                        fail(
                            Prop::Deterministic,
                            r.id(),
                            format!(
                                "second call differs: {rec:?} vs {rec2:?}\n    first:  {}\n    \
                                 second: {}",
                                json(&spec),
                                json(&spec2)
                            ),
                        );
                    }
                }
            }
        }
    }

    // Through the registry (wrapper stripping, dispatch, fallbacks, contract
    // enforcement).
    let via = || {
        let mut spec = TaskSpec::default();
        let rep = registry().recognize(&case.cmd, &cx, &mut spec);
        spec.normalize();
        (rep, spec)
    };
    let (a, ta) = timed(via);
    match a {
        Err(e) => fail(
            Prop::NoPanic,
            "registry",
            format!("Registry::recognize panicked: {e}"),
        ),
        Ok((rep, spec)) => {
            let who = rep.recognizer.unwrap_or("registry").to_string();
            if ta > budget {
                fail(
                    Prop::BoundedTime,
                    &who,
                    format!("Registry::recognize took {ta:?} (budget {budget:?})"),
                );
            }
            for (p, m) in check_paths(&spec, &repo.root, confined) {
                fail(p, &who, m);
            }
            if let Some(m) = check_full(rep.recognition, &spec) {
                fail(Prop::FullNotUnknown, &who, m);
            }
            if let (Ok((rep2, spec2)), _) = timed(via)
                && (rep2 != rep || spec2 != spec)
            {
                fail(
                    Prop::Deterministic,
                    &who,
                    format!(
                        "second call differs: {rep:?} vs {rep2:?}\n    first:  {}\n    second: {}",
                        json(&spec),
                        json(&spec2)
                    ),
                );
            }
        }
    }
    fails
}

fn json(spec: &TaskSpec) -> String {
    serde_json::to_string(spec).unwrap_or_default()
}

/// A program name close to `p` that is (usually) not declared by anyone.
pub fn near_miss(p: &str, rng: &mut Rng) -> String {
    match rng.below(9) {
        0 => format!("{p}3.11"),
        1 => format!("{p}.{}", rng.below(20)),
        2 => p.to_uppercase(),
        3 => format!("{p}-x"),
        4 => format!("x{p}"),
        5 => format!("{p}\u{A0}"),
        6 => p
            .chars()
            .take(p.chars().count().saturating_sub(1))
            .collect(),
        7 => format!("{p}{}", rng.below(10)),
        _ => format!("{p}_"),
    }
}

/// Generates `per_program` cases in `mode` for every declared program of every
/// registered recognizer and checks them. Returns the violations (deduplicated
/// per recognizer and property, first input kept) and the number of cases run.
pub fn run_registry(mode: Mode, per_program: usize) -> (Vec<Failure>, usize) {
    let repo = SyntheticRepo::new();
    let mut fails: Vec<Failure> = Vec::new();
    let mut n = 0;
    for r in registry().recognizers() {
        let mut g = Gen::new(
            &repo,
            seed() ^ fnv1a(r.id()) ^ (mode as u64).wrapping_mul(0x9E37),
        );
        for program in r.programs() {
            for i in 0..per_program {
                // Every fourth command uses a near-miss of the program name (`python3.11`,
                // `TSC`, `tsc-x`): no recognizer may claim a program it does not declare.
                let program = if i % 4 == 3 {
                    near_miss(program, &mut g.rng)
                } else {
                    (*program).to_string()
                };
                let case = g.case(&program, mode);
                n += 1;
                for f in check_case(*r, &case, &repo) {
                    if !fails
                        .iter()
                        .any(|x| x.prop == f.prop && x.recognizer == f.recognizer)
                    {
                        fails.push(f);
                    }
                }
            }
        }
    }
    (fails, n)
}

/// Panics with a readable report when `fails` is not empty.
pub fn assert_no_failures(what: &str, fails: &[Failure], cases: usize) {
    if fails.is_empty() {
        return;
    }
    let mut s = format!(
        "{what}: {} violation(s) over {cases} generated commands (seed {:#x}; set \
         TURBO_TRACE_DEBUG_TESTKIT_SEED to vary)\n",
        fails.len(),
        seed()
    );
    for f in fails {
        let _ = writeln!(s, "- [{:?}] {}: {}", f.prop, f.recognizer, f.message);
    }
    panic!("{s}");
}

// ---- Per-tool fixtures
// -------------------------------------------------------------------------

/// The directory that holds the shared `fixtures/` (the plugin testkit crate).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../turborepo-trace-testkit")
        .canonicalize()
        .expect("fixtures root")
}

fn rel(s: String, root: &Path) -> String {
    s.replace(&format!("{}/", root.display()), "")
        .replace(&root.display().to_string(), ".")
}

/// Commands of `<dir>/commands.txt` (blank lines and `#` comments skipped).
pub fn commands_file(dir: &str) -> Vec<String> {
    let p = workspace_root().join(dir).join("commands.txt");
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Recognizes each command line in the fixture directory `dir` (relative to the
/// workspace root; it is both repo and package root) and renders the recognized
/// `TaskSpec`s with paths relative to the fixture. Checks the invariants
/// (absolute normalized paths, `Full` without unknown commands, deterministic)
/// on every line and panics on a violation.
pub fn fixture_text<S: AsRef<str>>(dir: &str, commands: &[S]) -> String {
    let root = workspace_root().join(dir);
    let root = root
        .canonicalize()
        .unwrap_or_else(|e| panic!("fixture dir {}: {e}", root.display()));
    let scripts: Vec<String> =
        turborepo_trace_tasks::sources::read_json(&root.join("package.json"))
            .and_then(|m| {
                m.get("scripts")
                    .and_then(|s| s.as_object())
                    .map(|o| o.keys().cloned().collect())
            })
            .unwrap_or_default();
    let env = BTreeMap::new();
    let run = |line: &str| -> (Vec<CommandReport>, TaskSpec) {
        let parsed = shell::parse(line, &root, &env);
        let mut spec = TaskSpec::default();
        spec.uncertainties.extend(parsed.uncertainties.clone());
        let cx = RecognizeCx {
            repo_root: &root,
            package_root: &root,
            env: &env,
            task_name: None,
            scripts: &scripts,
        };
        let reps = parsed
            .commands
            .iter()
            .map(|c| registry().recognize(c, &cx, &mut spec))
            .collect();
        spec.normalize();
        (reps, spec)
    };
    let mut out = String::new();
    for line in commands {
        let line = line.as_ref();
        let (reps, spec) = run(line);
        let again = run(line);
        assert!(
            again == (reps.clone(), spec.clone()),
            "`{line}`: recognition is not deterministic"
        );
        if let Some((p, m)) = check_paths(&spec, &root, false).into_iter().next() {
            panic!("`{line}`: [{p:?}] {m}");
        }
        for r in &reps {
            // Per command: a Full command must not be the one that added an unknown
            // command.
            if r.recognition == Recognition::Full
                && let Some(p) = &r.program
                && spec.uncertainties.iter().any(|u| {
                    u.kind == UncertaintyKind::UnknownCommand
                        && (u.detail == *p || u.detail.contains(&format!("`{p}`")))
                })
            {
                panic!("`{line}`: `{p}` is Full but also an unknown command");
            }
        }
        let recs: Vec<String> = reps
            .iter()
            .map(|r| {
                format!(
                    "{} [{}] {:?}",
                    r.program.clone().unwrap_or_default(),
                    r.recognizer.unwrap_or("-"),
                    r.recognition
                )
            })
            .collect();
        let json = serde_json::to_string_pretty(&spec).expect("json");
        let _ = writeln!(
            out,
            "$ {line}\n# {}\n{}\n",
            recs.join(" | "),
            rel(json, &root)
        );
    }
    out
}

/// A per-tool recognizer fixture: recognizes commands in a fixture directory
/// and snapshots the resulting `TaskSpec`s (insta, `tests/snapshots/<test
/// file>__<name>.snap`).
///
/// ```ignore
/// #[macro_use]
/// mod testkit;
///
/// // Inline commands; the directory (relative to the workspace root) is the repo and package root.
/// recognizer_fixture!(tsc_basic, "fixtures/tasks/recognizers/ts", ["tsc -p tsconfig.build.json", "tsc --noEmit"]);
/// // Or one command per line from `<dir>/commands.txt`.
/// recognizer_fixture!(ts_family, "fixtures/tasks/recognizers/ts");
/// ```
macro_rules! recognizer_fixture {
    ($name:ident, $dir:expr, [$($cmd:expr),* $(,)?]) => {
        #[test]
        fn $name() {
            let commands: &[&str] = &[$($cmd),*];
            insta::assert_snapshot!(stringify!($name), $crate::testkit::fixture_text($dir, commands));
        }
    };
    ($name:ident, $dir:expr) => {
        #[test]
        fn $name() {
            let commands = $crate::testkit::commands_file($dir);
            insta::assert_snapshot!(stringify!($name), $crate::testkit::fixture_text($dir, &commands));
        }
    };
}
