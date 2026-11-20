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
//! - the call's options do not name `cwd`, and the file does not call
//!   `process.chdir`: the command runs where the bin was started.
//!
//! The environment the bin passes on is not read (as for `env X=1`). A bin
//! that picks the program from a table, wraps it in a shell string, or builds
//! the arguments in a loop is not a forwarder.

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
    if options.iter().any(|o| o.contains("cwd")) {
        return false;
    }
    let count = |name: &str| occurrences(&code, name).len();
    if *program == "process.argv[2]" && *rest == "process.argv.slice(3)" {
        return true;
    }
    if is_name(program) && is_name(rest) && program != rest {
        let declared = [
            format!("[{program},...{rest}]=process.argv.slice(2)"),
            format!("[,,{program},...{rest}]=process.argv"),
        ]
        .iter()
        .any(|d| code.contains(d.as_str()))
            || (code.contains(&format!("{program}=process.argv[2]"))
                && code.contains(&format!("{rest}=process.argv.slice(3)")));
        // The rest: its declaration and the call.
        return declared && count(rest) == 2;
    }
    if let Some(list) = program.strip_suffix("[0]")
        && is_name(list)
        && *rest == format!("{list}.slice(1)")
    {
        // The list: its declaration and the two uses in the call.
        return code.contains(&format!("{list}=process.argv.slice(2)")) && count(list) == 3;
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
             process.argv.slice(2);\nconsole.log(program);\nspawn(program, rest, { stdio: \
             'inherit', env: { ...process.env, CI: '1' } }).on('exit', (c) => process.exit(c));",
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
}
