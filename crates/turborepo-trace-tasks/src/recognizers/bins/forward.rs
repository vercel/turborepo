//! First-party binaries that forward their arguments to the program named
//! first: `tool-run node --test x` starts `node --test x`, as `env X=1 node
//! --test x` or `timeout 600 node --test x` do. Such a bin is a prefix: the
//! task is recognized by the command behind it.
//!
//! The decision is made from the source of the bin's entry file, never from
//! its name, and only for these shapes (comments and whitespace aside):
//!
//! | The program and its arguments | Example |
//! |---|---|
//! | `process.argv[2]` and `process.argv.slice(3)` | `spawnSync(process.argv[2], process.argv.slice(3), { stdio: 'inherit' })` |
//! | a name and a rest taken apart from `process.argv.slice(2)` | `const [program, ...rest] = process.argv.slice(2); spawn(program, rest)` |
//! | a name and a rest taken apart from `process.argv` | `const [, , program, ...rest] = process.argv; spawn(program, rest)` |
//! | two names bound to `process.argv[2]` and `process.argv.slice(3)` | `const program = process.argv[2]; const rest = process.argv.slice(3); spawn(program, rest)` |
//! | the first element and the rest of a name bound to `process.argv.slice(2)` | `const args = process.argv.slice(2); spawn(args[0], args.slice(1))` |
//!
//! as the first two arguments of the one process the file starts: a call of
//! `spawn`, `spawnSync`, `execFile`, `execFileSync`, `execa`, `execaSync`,
//! `crossSpawn` or `spawn.sync`. All of this must hold:
//!
//! - the file starts no other process (no second call of these, no `exec`,
//!   `execSync`, `fork` or `$` call): a bin that also runs a tool of its own is
//!   read as before ([`super::wrapper`]);
//! - the name of the rest is used nowhere else (nothing can add or drop an
//!   argument), and neither is a name bound to `process.argv.slice(2)`;
//! - the name of the program is used nowhere else either, and `process.argv` is
//!   named by the shape alone;
//! - the call has no options, or an object written in the call with keys among
//!   `stdio`, `windowsHide`, `timeout`, `killSignal`, `encoding` and
//!   `maxBuffer`: no `cwd`, no `env`, no spread, no name set elsewhere;
//! - the file does not call `process.chdir`, does not name `process.env`, reads
//!   no file, and imports nothing but the module that starts the process
//!   (`child_process`, `cross-spawn`, `execa`, `process`): a module of its
//!   package could set a preload in the environment;
//! - `process` is named only for what the shape needs and for how the bin ends
//!   (`process.argv`, `process.exit`, `process.exitCode`, `process.on`,
//!   `process.kill`, `process.pid`, the standard streams), and the `process`
//!   module is loaded only as that object (`import process from
//!   'node:process'`, `const process = require('process')`): its parts under
//!   other names (`import { env, chdir } from 'node:process'`, `const { env } =
//!   process`) change the environment or the directory without naming
//!   `process.env` or `process.chdir`. Neither is `globalThis` or `global`
//!   named.
//!
//! So the command behind the bin runs with the bin's own arguments,
//! environment and working directory. A bin that picks the program from a
//! table, wraps it in a shell string, builds the arguments in a loop or
//! passes an environment of its own is not a forwarder.

use super::wrapper::Lexed;

/// Calls that start a process with a program and an argument list.
const SPAWNS: &[&str] = &[
    "spawn(",
    "spawnSync(",
    "execFile(",
    "execFileSync(",
    "execa(",
    "execaSync(",
    "crossSpawn(",
    "spawn.sync(",
    "crossSpawn.sync(",
];

/// Calls that start a process in another way.
const OTHER_STARTS: &[&str] = &[
    "exec(",
    "execSync(",
    "fork(",
    "execaCommand(",
    "execaCommandSync(",
    "$(",
    "Bun.spawn(",
    "Bun.spawnSync(",
];

/// The keys an options object of the call may have: none of them changes the
/// program, its arguments, its environment or where it runs.
const OPTION_KEYS: &[&str] = &[
    "stdio",
    "windowsHide",
    "timeout",
    "killSignal",
    "encoding",
    "maxBuffer",
];

/// The modules the file may load: the ones that start the process.
const MODULES: &[&str] = &[
    "child_process",
    "node:child_process",
    "cross-spawn",
    "execa",
    "process",
    "node:process",
];

/// The modules of [`MODULES`] that are the `process` object.
const PROCESS_MODULES: &[&str] = &["process", "node:process"];

/// What may follow the name `process`: its arguments, and how the bin ends.
const PROCESS_USES: &[&str] = &[
    ".argv",
    ".exit(",
    ".exitCode",
    ".on(",
    ".once(",
    ".kill(",
    ".pid",
    ".stdout",
    ".stderr",
    ".stdin",
];

/// The ways the `process` module is loaded as the object of that name (in
/// the code without whitespace, a string literal is `""`).
const PROCESS_LOADS: &[&str] = &["import process from\"\"", "process=require(\"\")"];

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// The positions in `code` where `word` starts as a whole name (a call such as
/// `spawn(` may follow a `.`).
fn occurrences(code: &str, word: &str) -> Vec<usize> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = code[from..].find(word) {
        let at = from + i;
        let end = at + word.len();
        let before = at == 0 || !is_ident(b[at - 1]);
        let last = word.as_bytes()[word.len() - 1];
        let after = !is_ident(last) || end >= b.len() || !is_ident(b[end]);
        if before && after {
            out.push(at);
        }
        from = end;
    }
    out
}

/// The arguments of the call whose `(` is at `open`: their texts, split at the
/// commas of the call itself.
fn call_args(code: &str, open: usize) -> Option<Vec<&str>> {
    let b = code.as_bytes();
    let mut depth = 0usize;
    let mut start = open + 1;
    let mut out = Vec::new();
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    out.push(&code[start..i]);
                    return Some(out);
                }
            }
            b',' if depth == 1 => {
                out.push(&code[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    None
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(is_ident)
        && !s.as_bytes()[0].is_ascii_digit()
        && !matches!(s, "process" | "undefined" | "null")
}

/// Whether the lexed entry file of a bin forwards its arguments to the
/// program named by the first one (module docs).
pub fn forwards(lx: &Lexed) -> bool {
    // (String contents are gone from `code`, comments too.)
    // Whitespace goes, except between two names (`const args`).
    let mut code = String::with_capacity(lx.code.len());
    let mut gap = false;
    for c in lx.code.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        let name = |c: char| c.is_ascii() && is_ident(c as u8);
        if gap && name(c) && code.chars().next_back().is_some_and(name) {
            code.push(' ');
        }
        gap = false;
        code.push(c);
    }
    if code.contains("process.chdir(") {
        return false;
    }
    // `process` is the global object, used for what the shape needs: no part
    // of it has another name.
    let loads: usize = PROCESS_LOADS.iter().map(|l| code.matches(l).count()).sum();
    let process_modules = lx
        .lits
        .iter()
        .filter(|l| PROCESS_MODULES.contains(&l.value.as_str()))
        .count();
    if process_modules != loads {
        return false;
    }
    for at in occurrences(&code, "process") {
        let (before, after) = (&code[..at], &code[at + "process".len()..]);
        let used = PROCESS_USES.iter().any(|u| after.starts_with(u));
        let loaded = (before.ends_with("import ") && after.starts_with(" from\"\""))
            || after.starts_with("=require(\"\")");
        if !(used || loaded) {
            return false;
        }
    }
    if !occurrences(&code, "globalThis").is_empty() || !occurrences(&code, "global").is_empty() {
        return false;
    }
    let mut calls: Vec<usize> = Vec::new();
    for s in SPAWNS {
        // `spawn.sync(` is no call of `spawn(`; `x.spawn(` is one.
        calls.extend(occurrences(&code, s).into_iter().map(|at| at + s.len() - 1));
    }
    let others = OTHER_STARTS
        .iter()
        .any(|s| !occurrences(&code, s).is_empty());
    if calls.len() != 1 || others {
        return false;
    }
    let Some(args) = call_args(&code, calls[0]) else {
        return false;
    };
    let [program, rest, options @ ..] = args.as_slice() else {
        return false;
    };
    // The options: none, or an object written in the call whose keys are all
    // known to leave the command alone (no `cwd`, no `env`, no spread, no name
    // whose value is set elsewhere).
    match options {
        [] => {}
        [object] => {
            let Some(inner) = object.strip_prefix('{').and_then(|o| o.strip_suffix('}')) else {
                return false;
            };
            let plain = inner.split(',').filter(|m| !m.is_empty()).all(|member| {
                member
                    .split_once(':')
                    .is_some_and(|(key, _)| OPTION_KEYS.contains(&key))
            });
            if !plain {
                return false;
            }
        }
        _ => return false,
    }
    // The file reads nothing and loads nothing of its own: an env file it
    // reads, or a module of its package that it imports, can change what the
    // program loads.
    if lx.lits.iter().any(|l| {
        !MODULES.contains(&l.value.as_str())
            && (l.ctx.ends_with("from")
                || l.ctx.ends_with("import")
                || l.ctx.ends_with("import(")
                || l.ctx.ends_with("require("))
    }) {
        return false;
    }
    if ["process.env", "readFile", "loadEnvFile", "createReadStream"]
        .iter()
        .any(|w| code.contains(w))
    {
        return false;
    }
    let count = |name: &str| occurrences(&code, name).len();
    // `process.argv` is named by the shape alone: nothing else reads or changes
    // it.
    let argv = count("process.argv");
    if *program == "process.argv[2]" && *rest == "process.argv.slice(3)" {
        return argv == 2;
    }
    if is_name(program) && is_name(rest) && program != rest {
        let together = [
            format!("[{program},...{rest}]=process.argv.slice(2)"),
            format!("[,,{program},...{rest}]=process.argv"),
        ]
        .iter()
        .any(|d| code.contains(d.as_str()));
        let apart = code.contains(&format!("{program}=process.argv[2]"))
            && code.contains(&format!("{rest}=process.argv.slice(3)"));
        // The program and the rest: the declaration and the call.
        return (together && argv == 1 || apart && argv == 2)
            && count(program) == 2
            && count(rest) == 2;
    }
    if let Some(list) = program.strip_suffix("[0]")
        && is_name(list)
        && *rest == format!("{list}.slice(1)")
    {
        // The list: its declaration and the two uses in the call.
        return code.contains(&format!("{list}=process.argv.slice(2)"))
            && count(list) == 3
            && argv == 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{super::wrapper::lex, *};

    #[test]
    fn the_shapes_that_forward() {
        for src in [
            "import { spawnSync } from 'node:child_process';\nconst r = \
             spawnSync(process.argv[2], process.argv.slice(3), { stdio: 'inherit' \
             });\nprocess.exit(r.status ?? 1);",
            "const { spawn } = require('child_process');\nconst [program, ...rest] = \
             process.argv.slice(2);\nspawn(program, rest, { stdio: 'inherit', windowsHide: true \
             }).on('exit', (c) => process.exit(c));",
            "const [, , cmd, ...args] = \
             process.argv;\nrequire('cross-spawn').sync;\ncrossSpawn.sync(cmd, args, { stdio: \
             'inherit' });",
            "const program = process.argv[2];\nconst rest = process.argv.slice(3);\n// \
             spawn('jest')\nchild_process.execFileSync(program, rest, { stdio: 'inherit' });",
            "const args = process.argv.slice(2);\nawait execa(args[0], args.slice(1), { stdio: \
             'inherit' });",
        ] {
            assert!(forwards(&lex(src)), "{src}");
        }
    }

    #[test]
    fn what_is_not_a_forwarder() {
        for src in [
            // An environment of its own, a module of its package, the program used again.
            "const [program, ...rest] = process.argv.slice(2);\nspawn(program, rest, { stdio: \
             'inherit', env: { ...process.env, CI: '1' } });",
            "import { environment } from '../src/environment.mjs';\nconst [program, ...rest] = \
             process.argv.slice(2);\nspawnSync(program, rest, { stdio: 'inherit', env: \
             environment() });",
            "const [program, ...rest] = \
             process.argv.slice(2);\nconsole.log(program);\nspawnSync(program, rest);",
            "import fs from 'node:fs';\nspawnSync(process.argv[2], process.argv.slice(3), { \
             stdio: 'inherit' });",
            // A tool of its own.
            "spawnSync('jest', process.argv.slice(2), { stdio: 'inherit' });",
            // A second process.
            "const [program, ...rest] = process.argv.slice(2);\nspawnSync('tsc', \
             ['-b']);\nspawnSync(program, rest);",
            "execSync('pnpm build');\nspawnSync(process.argv[2], process.argv.slice(3));",
            // The arguments are changed.
            "const [program, ...rest] = \
             process.argv.slice(2);\nrest.push('--ci');\nspawnSync(program, rest);",
            "const args = process.argv.slice(2);\nargs.unshift('x');\nspawn(args[0], \
             args.slice(1));",
            "const [program, ...rest] = process.argv.slice(2);\nspawnSync(program, ['--ci', \
             ...rest]);",
            // Another directory.
            "spawnSync(process.argv[2], process.argv.slice(3), { cwd: root });",
            "process.chdir(root);\nspawnSync(process.argv[2], process.argv.slice(3));",
            // The program is picked from a table, or is not the first argument.
            "const [name, ...rest] = process.argv.slice(2);\nspawnSync(tools[name], rest);",
            "spawnSync(process.argv[3], process.argv.slice(4));",
            "const [program, ...rest] = process.argv.slice(3);\nspawnSync(program, rest);",
            // A shell string.
            "const [program, ...rest] = process.argv.slice(2);\nexecSync(`${program} \
             ${rest.join(' ')}`);",
            // Nothing is started.
            "console.log(process.argv[2], process.argv.slice(3));",
        ] {
            assert!(!forwards(&lex(src)), "{src}");
        }
    }

    #[test]
    fn bins_that_change_what_they_forward() {
        for src in [
            // The options are a name: its `cwd` is not seen.
            "const options = { stdio: 'inherit', cwd: path.resolve(__dirname, '../../..') \
             };\nconst [program, ...rest] = process.argv.slice(2);\nspawnSync(program, rest, \
             options);",
            // The program is replaced after it is read.
            "let program = process.argv[2];\nconst rest = process.argv.slice(3);\nif (program === \
             'node') program = 'jest';\nspawnSync(program, rest, { stdio: 'inherit' });",
            // An argument is added to `process.argv` itself.
            "process.argv.push('extra/smoke.js');\nspawnSync(process.argv[2], \
             process.argv.slice(3), { stdio: 'inherit' });",
            // The environment gets a preload.
            "process.env.NODE_OPTIONS = '--import ' + hook;\nspawnSync(process.argv[2], \
             process.argv.slice(3), { stdio: 'inherit' });",
        ] {
            assert!(!forwards(&lex(src)), "{src}");
        }
    }

    #[test]
    fn the_process_object_under_its_own_name_forwards() {
        for src in [
            "import process from 'node:process';\nimport { spawnSync } from \
             'node:child_process';\nconst r = spawnSync(process.argv[2], process.argv.slice(3), { \
             stdio: 'inherit' });\nprocess.exitCode = r.status ?? 1;",
            "const process = require('process');\nconst { spawn } = \
             require('child_process');\nconst child = spawn(process.argv[2], \
             process.argv.slice(3), { stdio: 'inherit' });\nprocess.on('SIGINT', () => \
             child.kill('SIGINT'));\nchild.on('exit', (c) => process.exit(c));",
        ] {
            assert!(forwards(&lex(src)), "{src}");
        }
    }

    #[test]
    fn parts_of_process_under_other_names_are_no_forwarder() {
        for src in [
            // Named imports change the environment and the directory.
            "import { env, chdir } from 'node:process';\nimport { spawnSync } from \
             'node:child_process';\nenv.NODE_OPTIONS = '--import \
             ./hook.mjs';\nchdir('..');\nspawnSync(process.argv[2], process.argv.slice(3), { \
             stdio: 'inherit' });",
            "import { env } from 'process';\nenv.NODE_OPTIONS = '--require \
             ./hook.cjs';\nspawnSync(process.argv[2], process.argv.slice(3), { stdio: 'inherit' \
             });",
            "import * as proc from 'node:process';\nproc.chdir('..');\nspawnSync(process.argv[2], \
             process.argv.slice(3));",
            "const { env, chdir } = \
             require('node:process');\nchdir('..');\nspawnSync(process.argv[2], \
             process.argv.slice(3));",
            // The object is taken apart, or has another name.
            "const { env } = process;\nenv.NODE_OPTIONS = '--import \
             ./hook.mjs';\nspawnSync(process.argv[2], process.argv.slice(3));",
            "const p = process;\np.chdir('..');\nspawnSync(process.argv[2], \
             process.argv.slice(3));",
            "process['env'].NODE_OPTIONS = '--import ./hook.mjs';\nspawnSync(process.argv[2], \
             process.argv.slice(3));",
            "globalThis['process'].chdir('..');\nspawnSync(process.argv[2], \
             process.argv.slice(3));",
            "Object.assign(process.env, { NODE_OPTIONS: '--import ./hook.mjs' \
             });\nspawnSync(process.argv[2], process.argv.slice(3));",
        ] {
            assert!(!forwards(&lex(src)), "{src}");
        }
    }
}
