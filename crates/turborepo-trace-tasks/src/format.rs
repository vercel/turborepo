//! In-place formatters, and scripts that only format.
//!
//! A formatter in write mode (`prettier --write`, `oxfmt`, `biome format
//! --write`, `dprint fmt`) rewrites each file it is pointed at. What it writes
//! into a file is a function of that file, of the formatter's configuration and
//! of its version: no other file takes part. So when a task generates code and
//! then calls a formatting script (`"generate": "gen && pnpm -w fmt"`), the
//! generated files do not depend on the rest of what the formatter is pointed
//! at, which for a root formatting script is the whole repository.
//!
//! [`format_only`] recognizes such a script from its command line. A command
//! that runs it ([`crate::sources::npm::PackageManager`]) records the
//! dependency as *in place* ([`crate::spec::TaskDep::in_place`]), and a task
//! that has an output its consumers link to drops its in-place dependencies
//! ([`drop_in_place_deps`]): consumers of the generated files then reach the
//! generator's own inputs only.
//!
//! What this gives up, and what it keeps:
//!
//! - The formatter's configuration and version are no longer inputs of the
//!   caller's outputs. Formatting does not change what code does; a consumer
//!   that compares generated text byte for byte after a formatter upgrade is
//!   the cost.
//! - The formatting task itself keeps its file set: it is affected by every
//!   file it formats.
//! - A caller without linking outputs keeps the dependency (nothing links to
//!   it, and its own verdict stays "whatever the formatter reads").
//! - A check (`--check`, `--list-different`) keeps the dependency: the caller
//!   then fails on any unformatted file. So do linters with `--fix`, which
//!   report what they can't fix.

use std::path::Path;

use crate::{
    registry::{RecognizeCx, strip_wrappers},
    shell,
    sources::read_json,
    spec::{Command, TaskSpec},
};

/// How deep `pnpm <script>` chains are followed.
const MAX_DEPTH: usize = 3;

/// Whether `arg` switches a formatter from writing to checking.
pub fn is_check_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--check" | "-c" | "--list-different" | "-l" | "--no-write" | "--dry-run"
    )
}

/// Whether `cmd` (wrappers stripped) runs formatter `prog` in write mode: it
/// rewrites the files it is given and reports nothing about their contents.
pub fn formats_in_place(prog: &str, cmd: &Command) -> bool {
    let has = |flags: &[&str]| cmd.args().any(|a| flags.contains(&a));
    if cmd.args().any(is_check_flag) {
        return false;
    }
    match prog {
        "prettier" => has(&["--write", "-w"]),
        // oxfmt writes unless told to check.
        "oxfmt" => true,
        "biome" => cmd.args().find(|a| !a.starts_with('-')) == Some("format") && has(&["--write"]),
        "dprint" => cmd.args().find(|a| !a.starts_with('-')) == Some("fmt"),
        _ => false,
    }
}

/// Whether every command of the script line `line`, run in `dir`, is an
/// in-place formatter ([`formats_in_place`]) or a call of a script of the same
/// package that is. Unsupported shell syntax, redirections and any other
/// command make it `false`.
pub fn format_only(line: &str, dir: &Path, cx: &RecognizeCx<'_>) -> bool {
    format_only_at(line, dir, cx, 0)
}

fn format_only_at(line: &str, dir: &Path, cx: &RecognizeCx<'_>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let parsed = shell::parse(line, dir, cx.env);
    if !parsed.uncertainties.is_empty() || parsed.commands.is_empty() {
        return false;
    }
    parsed.commands.iter().all(|c| {
        if !c.redirects.is_empty() {
            return false;
        }
        // The script of the same package that a package-manager command runs, before
        // wrappers are stripped (`pnpm fmt:all` is not a wrapper around a program).
        if let Some(name) = local_script(c)
            && let Some(text) = script_text(dir, name)
        {
            return !c.args().any(is_check_flag) && format_only_at(&text, dir, cx, depth + 1);
        }
        let mut scratch = TaskSpec::default();
        let real = strip_wrappers(c, cx, &mut scratch);
        real.program_name()
            .is_some_and(|p| formats_in_place(p, &real))
    })
}

/// The script name of `pnpm <script> …`, `yarn <script> …`, `npm run <script>
/// …` with no option that sends it to another package.
fn local_script(cmd: &Command) -> Option<&str> {
    let pm = cmd.program_name()?;
    if !matches!(pm, "pnpm" | "yarn" | "npm" | "bun") {
        return None;
    }
    let mut args = cmd.args();
    let first = args.next()?;
    if first.starts_with('-') {
        return None;
    }
    match first {
        "run" | "run-script" => args.next().filter(|a| !a.starts_with('-')),
        s if pm != "npm" => Some(s),
        _ => None,
    }
}

/// The command line of script `name` in the package at `dir`.
pub fn script_text(dir: &Path, name: &str) -> Option<String> {
    read_json(&dir.join("package.json"))?
        .get("scripts")?
        .get(name)?
        .as_str()
        .map(str::to_owned)
}

/// Removes the in-place dependencies of a task that produces outputs (see the
/// module docs).
pub fn drop_in_place_deps(spec: &mut TaskSpec) {
    if spec.task_deps.iter().any(|d| d.in_place) && spec.outputs.iter().any(|o| o.links_producer())
    {
        spec.task_deps.retain(|d| !d.in_place);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Env;

    fn only(line: &str) -> bool {
        let env = Env::new();
        let scripts: Vec<String> = Vec::new();
        let root = Path::new("/r");
        let cx = RecognizeCx {
            repo_root: root,
            package_root: root,
            env: &env,
            task_name: None,
            scripts: &scripts,
        };
        format_only(line, root, &cx)
    }

    #[test]
    fn formatters_in_write_mode() {
        for line in [
            "oxfmt --write --ignore-path .gitignore --no-error-on-unmatched-pattern",
            "oxfmt .",
            "prettier --write .",
            "prettier -w src && oxfmt",
            "pnpm exec prettier --write 'src/**/*.ts'",
            "npx prettier --write .",
            "biome format --write .",
            "dprint fmt",
        ] {
            assert!(only(line), "{line}");
        }
    }

    #[test]
    fn checks_linters_and_other_commands_are_not() {
        for line in [
            "oxfmt --check .",
            "oxfmt --list-different",
            "prettier .",
            "prettier --check .",
            "prettier --write . --check",
            "biome check --write .",
            "biome format .",
            "dprint check",
            "eslint --fix .",
            "oxlint --fix",
            "prettier --write . && node scripts/gen.js",
            "prettier --write . > log.txt",
            "prettier --write $FILES",
            "",
        ] {
            assert!(!only(line), "{line}");
        }
    }
}
