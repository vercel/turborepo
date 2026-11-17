//! Static wrapper detection for first-party binaries: does a bin run a known
//! tool?
//!
//! Many internal binaries wrap a tool (`jest-dd` runs jest with Datadog
//! instrumentation, `tsc-build` runs tsc). A cheap lexical scan of the bin
//! entry and its first-party relative imports (inside the bin's package, at
//! most [`MAX_FILES`] files) collects string literals with the code just before
//! them, and decides:
//!
//! | Evidence | Strength |
//! |---|---|
//! | a tool name as the program of a spawn-like call: `spawn('jest', …)`, `execa('vitest', args)`, `execSync('npx tsc -p x')` | strong |
//! | a tool package's CLI entry loaded or resolved: `require('jest').run(…)`, `require.resolve('jest/bin/jest')`, `import { startVitest } from 'vitest/node'` (library use such as `require('esbuild').build(…)` does not count) | strong |
//! | a bare tool-name literal (`runner: 'jest'`) while some scanned file launches processes (`child_process`, `execa`, `cross-spawn`, `spawn(`) | weak |
//!
//! Tiers, first non-empty wins: strong in the entry file, weak in the entry
//! file, strong in the rest of the closure, weak in the rest. A tier with
//! several tools forwards to each (at most 3). A "tool" is a program some
//! registered recognizer handles, minus package managers, script runners
//! (`node`, `tsx`: their first argument would be the bin's argument), file
//! utilities and words too generic to be evidence.
//!
//! The same scan flags reads relative to the working directory
//! (`process.cwd()`, `INIT_CWD`, `path.resolve('<literal>')`), which the caller
//! covers with a membership.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::{run::package_name, util::resolve_js_file},
    registry::{WRAPPERS, registry},
};

/// Files scanned per bin.
pub const MAX_FILES: usize = 12;
/// Bytes scanned per file.
const MAX_BYTES: usize = 512 * 1024;

/// What the scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// Tools the bin runs (registry program names), in tier order.
    pub tools: Vec<&'static str>,
    /// Whether the evidence was only weak.
    pub weak: bool,
    /// Whether some scanned file reads paths relative to the cwd.
    pub cwd: bool,
    /// The files scanned (entry first).
    pub files: Vec<PathBuf>,
}

/// A string literal with the code right before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lit {
    /// Value (escapes kept as written; templates with `${}` are skipped).
    pub value: String,
    /// Up to 48 bytes of code before the literal, trailing whitespace removed.
    pub ctx: String,
    /// Up to 16 bytes of source after the literal, whitespace removed.
    pub after: String,
}

/// A lexed JS file: literals, plus the code with comments and string contents
/// removed.
#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// String literals.
    pub lits: Vec<Lit>,
    /// Code without comments; literals replaced by `""`.
    pub code: String,
}

/// Lexes JS/TS source well enough for literal scanning (no regex-literal
/// detection).
pub fn lex(src: &str) -> Lexed {
    let b = src.as_bytes();
    let mut out = Lexed::default();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if matches!(c, b'\'' | b'"' | b'`') {
            let start = i + 1;
            let mut j = start;
            let mut interp = false;
            while j < b.len() && b[j] != c {
                if b[j] == b'\\' {
                    j += 1;
                } else if c == b'`' && b[j] == b'$' && b.get(j + 1) == Some(&b'{') {
                    interp = true;
                } else if c != b'`' && b[j] == b'\n' {
                    break;
                }
                j += 1;
            }
            let end = j.min(b.len());
            if !interp {
                let ctx_start = out.code.len().saturating_sub(48);
                let mut cs = ctx_start;
                while !out.code.is_char_boundary(cs) {
                    cs += 1;
                }
                let after: String = src
                    .get(end + 1..)
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .take(16)
                    .collect();
                out.lits.push(Lit {
                    value: src[start..end].to_string(),
                    ctx: out.code[cs..].trim_end().to_string(),
                    after,
                });
            }
            out.code.push_str("\"\"");
            i = end + 1;
            continue;
        }
        // Copy one UTF-8 char.
        let len = utf8_len(c);
        let e = (i + len).min(b.len());
        out.code.push_str(&src[i..e]);
        i = e;
    }
    out
}

fn utf8_len(c: u8) -> usize {
    match c {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

/// Programs that count as tools (see the module docs).
pub fn tools() -> &'static BTreeSet<&'static str> {
    static T: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    T.get_or_init(|| {
        const DENY: &[&str] = &[
            "node", "nodejs", "bun", "deno", "npm", "pnpm", "yarn", "npx", "pnpx", "bunx",
            "corepack", "python", "python3", "python2", "py", "pypy3", "go", "cargo", "sh", "bash",
            "zsh", "git", "make", "docker", "turbo", "nx", "lerna", "standard", "tap", "lab",
            "tape", "xo", "uvu", "test", "build", "run", "start", "dev", "lint",
        ];
        let mut deny: BTreeSet<&str> = DENY.iter().copied().collect();
        deny.extend(WRAPPERS.iter().copied());
        for r in crate::recognizers::run::recognizers()
            .into_iter()
            .chain(crate::recognizers::fs::recognizers())
        {
            deny.extend(r.programs().iter().copied());
        }
        registry()
            .recognizers()
            .iter()
            .flat_map(|r| r.programs().iter().copied())
            .filter(|p| !deny.contains(p))
            .collect()
    })
}

/// The tool a loaded package provides (`typescript` → `tsc`, `@playwright/test`
/// → `playwright`), if any.
pub fn tool_of_package(pkg: &str) -> Option<&'static str> {
    let mapped = match pkg {
        "typescript" | "@typescript/native-preview" => "tsc",
        p if p.starts_with("@typescript/typescript") => "tsc",
        "@playwright/test" | "playwright" | "playwright-core" => "playwright",
        "jest-cli" | "@jest/core" => "jest",
        "@biomejs/biome" => "biome",
        "webpack-cli" => "webpack",
        "@rspack/cli" | "@rspack/core" => "rspack",
        "@graphql-codegen/cli" => "graphql-codegen",
        "@storybook/cli" => "storybook",
        p => p,
    };
    tools().get(mapped).copied()
}

/// The tool a command string runs (`'jest'`, `'npx tsc -p x'`, `'pnpm exec
/// vitest run'`).
fn tool_of_command(s: &str) -> Option<&'static str> {
    let prog = s.split_whitespace().find(|w| {
        !matches!(
            *w,
            "npx" | "pnpx" | "bunx" | "pnpm" | "yarn" | "exec" | "bun" | "x"
        )
    })?;
    let prog = prog.rsplit('/').next().unwrap_or(prog);
    tools().get(prog).copied()
}

const SPAWN_FNS: &[&str] = &[
    "spawn(",
    "spawnSync(",
    "exec(",
    "execSync(",
    "execFile(",
    "execFileSync(",
    "fork(",
    "execa(",
    "execaSync(",
    "execaCommand(",
    "execaCommandSync(",
    "crossSpawn(",
    "Bun.spawn(",
    "Bun.spawnSync(",
    "$(",
];

/// Programmatic CLI entry points of tool packages.
const CLI_APIS: &[&str] = &["runCLI(", "startVitest(", "createVitest("];

const LOADERS: &[&str] = &[
    "require(",
    "require.resolve(",
    "import.meta.resolve(",
    "import(",
    "from",
    "import",
];

/// Whether `ctx` (code before a literal) ends with one of `words`, at a token
/// boundary.
fn ends_with_any(ctx: &str, words: &[&str]) -> bool {
    words.iter().any(|w| {
        ctx.strip_suffix(w).is_some_and(|before| {
            !before
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
                || w.starts_with('.')
                || w.starts_with('$')
        })
    })
}

/// Tools found in one lexed file: (strong, weak).
fn file_tools(lx: &Lexed) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut strong = Vec::new();
    let mut weak = Vec::new();
    for l in &lx.lits {
        let in_spawn = ends_with_any(&l.ctx, SPAWN_FNS);
        let in_loader = ends_with_any(&l.ctx, LOADERS);
        if in_spawn {
            if let Some(t) = tool_of_command(&l.value) {
                strong.push(t);
            }
        } else if in_loader {
            if l.value.starts_with('.') || l.value.starts_with('/') {
                continue;
            }
            let pkg = package_name(&l.value);
            let Some(t) = tool_of_package(pkg) else {
                continue;
            };
            // Only a CLI entry counts, not library use (`require('esbuild').build(…)`).
            let resolve = l.ctx.ends_with("resolve(");
            let sub = &l.value[pkg.len()..];
            let cli_path = sub.contains("/bin")
                || sub.contains("cli")
                || sub.trim_start_matches('/').starts_with(t);
            let cli_call =
                l.after.starts_with(").run(") || CLI_APIS.iter().any(|a| lx.code.contains(a));
            if (resolve && cli_path) || (!resolve && (cli_path || cli_call)) {
                strong.push(t);
            }
        } else if let Some(t) = tools().get(l.value.as_str()) {
            weak.push(*t);
        }
    }
    (strong, weak)
}

/// Whether a lexed file launches processes.
fn launches(lx: &Lexed) -> bool {
    lx.lits.iter().any(|l| {
        ends_with_any(&l.ctx, LOADERS)
            && matches!(
                l.value.as_str(),
                "child_process" | "node:child_process" | "execa" | "cross-spawn" | "zx" | "zx/core"
            )
    }) || [
        "spawn(",
        "spawnSync(",
        "execSync(",
        "execFileSync(",
        "execa(",
        "Bun.spawn",
    ]
    .iter()
    .any(|s| lx.code.contains(s))
}

/// Whether a lexed file reads relative to the cwd.
fn reads_cwd(lx: &Lexed) -> bool {
    lx.code.contains("process.cwd")
        || lx.code.contains("INIT_CWD")
        || lx.lits.iter().any(|l| {
            l.ctx.ends_with("resolve(")
                && !l.ctx.ends_with("require.resolve(")
                && !l.ctx.ends_with("meta.resolve(")
        })
}

/// Relative imports of a lexed file, resolved against `dir`.
fn relative_imports(lx: &Lexed, dir: &Path) -> Vec<PathBuf> {
    lx.lits
        .iter()
        .filter(|l| {
            (l.value.starts_with("./") || l.value.starts_with("../"))
                && ends_with_any(&l.ctx, &["require(", "import(", "from", "import"])
        })
        .map(|l| resolve_js_file(dir, &l.value))
        .collect()
}

fn dedup(v: Vec<&'static str>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for t in v {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out.truncate(3);
    out
}

/// Scans the bin `entry` and its relative imports inside `pkg_dir`.
pub fn scan(entry: &Path, pkg_dir: &Path) -> Scan {
    let mut s = Scan::default();
    let mut todo = vec![entry.to_path_buf()];
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let (mut entry_strong, mut entry_weak) = (Vec::new(), Vec::new());
    let (mut rest_strong, mut rest_weak) = (Vec::new(), Vec::new());
    let mut launch = false;
    while let Some(f) = todo.pop() {
        if s.files.len() >= MAX_FILES || !seen.insert(f.clone()) {
            continue;
        }
        if !f.starts_with(pkg_dir) || !fsx::is_file(&f) {
            continue;
        }
        let Ok(bytes) = fsx::read(&f) else { continue };
        let src = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BYTES)]);
        let lx = lex(&src);
        let (st, wk) = file_tools(&lx);
        if s.files.is_empty() {
            entry_strong = st;
            entry_weak = wk;
        } else {
            rest_strong.extend(st);
            rest_weak.extend(wk);
        }
        launch |= launches(&lx);
        s.cwd |= reads_cwd(&lx);
        let dir = f.parent().unwrap_or(pkg_dir).to_path_buf();
        s.files.push(f);
        let mut next = relative_imports(&lx, &dir);
        next.reverse();
        todo.extend(next);
    }
    let tiers: [(Vec<&'static str>, bool); 4] = [
        (entry_strong, false),
        (if launch { entry_weak } else { Vec::new() }, true),
        (rest_strong, false),
        (if launch { rest_weak } else { Vec::new() }, true),
    ];
    for (t, weak) in tiers {
        if !t.is_empty() {
            s.tools = dedup(t);
            s.weak = weak;
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer() {
        let lx = lex("// 'no'\nconst a = require('./x'); /* \"no\" */ spawn(`jest`, [`${y}`]);");
        let v: Vec<(&str, &str)> = lx
            .lits
            .iter()
            .map(|l| (l.value.as_str(), l.ctx.as_str()))
            .collect();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "./x");
        assert!(v[0].1.ends_with("require("));
        assert_eq!(v[1].0, "jest");
        assert!(v[1].1.ends_with("spawn("));
        assert!(!lx.code.contains("no"));
    }

    #[test]
    fn evidence() {
        let lx = lex(
            "const { spawn } = require('child_process'); spawn('jest', process.argv.slice(2));"
        );
        assert_eq!(file_tools(&lx).0, ["jest"]);
        assert!(launches(&lx));
        let lx = lex(
            "require('jest').run(); require.resolve('vitest/vitest.mjs'); \
             require.resolve('@playwright/test/cli')",
        );
        assert_eq!(file_tools(&lx).0, ["jest", "vitest", "playwright"]);
        // Library use is not a CLI wrapper.
        let lx = lex("const esbuild = require('esbuild'); esbuild.build({}); import('typescript')");
        assert_eq!(file_tools(&lx), (vec![], vec![]));
        let lx = lex("import { startVitest } from 'vitest/node'; await startVitest('test', argv)");
        assert_eq!(file_tools(&lx).0, ["vitest"]);
        let lx = lex("run({ runner: 'jest', name: 'unit' })");
        assert_eq!(file_tools(&lx), (vec![], vec!["jest"]));
        assert!(!launches(&lx));
        let lx = lex("execSync('pnpm exec vitest run')");
        assert_eq!(file_tools(&lx).0, ["vitest"]);
        // Script runners and package managers are not tools.
        let lx = lex("spawn('node', [x]); spawn('pnpm', ['exec', r]); x.exec('tsx')");
        assert_eq!(file_tools(&lx).0, Vec::<&str>::new());
    }

    #[test]
    fn cwd_reads() {
        assert!(reads_cwd(&lex(
            "fs.readFileSync(path.join(process.cwd(), 'a'))"
        )));
        assert!(reads_cwd(&lex("path.resolve('openapi.json')")));
        assert!(!reads_cwd(&lex("require.resolve('dd-trace/ci/init')")));
        assert!(!reads_cwd(&lex("path.resolve(__dirname, 'a')")));
    }
}
