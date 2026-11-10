//! `package.json` `scripts` (with `pre*`/`post*` hooks) and the commands that
//! refer to scripts: `npm|pnpm|yarn|bun run x`, `pnpm -F pkg run x`, `yarn
//! workspace pkg x`, `npm-run-all`/`run-s`/`run-p`, `concurrently`, `lerna
//! run`.
//!
//! Every script is a task `task:<pkg>#<name>`. A `pre<name>`/`post<name>`
//! script is a task dependency of `<name>` (npm runs both around it; pnpm only
//! with `enable-pre-post-scripts`, so this over-approximates there, which is
//! sound).

use std::collections::BTreeMap;

use turborepo_trace_core::UncertaintyKind;

use crate::{
    recognizers::run::node_like,
    registry::{Recognition, RecognizeCx, Recognizer, is_pm_builtin, registry},
    sources::{DiscoveredTask, SourceCx, TaskSource, read_json},
    spec::{Command, EntryLang, TaskRef, TaskSpec},
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&NpmScripts]
}

/// Recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&PackageManager, &RunAll, &Concurrently, &Lerna]
}

/// `package.json` `scripts`.
pub struct NpmScripts;

impl TaskSource for NpmScripts {
    fn id(&self) -> &'static str {
        "npm"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["package.json"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let pj = cx.package.dir.join("package.json");
        let Some(manifest) = read_json(&pj) else {
            return Vec::new();
        };
        let Some(scripts) = manifest.get("scripts").and_then(|s| s.as_object()) else {
            return Vec::new();
        };
        let mut env = BTreeMap::new();
        // Package managers point `$npm_execpath` at themselves (`$npm_execpath run
        // build`).
        env.insert(
            "npm_execpath".to_string(),
            package_manager_of(&cx.workspace.root).to_string(),
        );
        if let Some(n) = manifest.get("name").and_then(|v| v.as_str()) {
            env.insert("npm_package_name".to_string(), n.to_string());
        }
        if let Some(v) = manifest.get("version").and_then(|v| v.as_str()) {
            env.insert("npm_package_version".to_string(), v.to_string());
        }
        let mut out = Vec::new();
        for (name, cmd) in scripts {
            let Some(line) = cmd.as_str() else { continue };
            let mut t =
                DiscoveredTask::new(cx.script_id(name), name.clone(), cx.package.dir.clone());
            t.lines.push(line.to_string());
            t.files.push(pj.clone());
            t.env = env.clone();
            t.env.insert("npm_lifecycle_event".into(), name.clone());
            for hook in ["pre", "post"] {
                let h = format!("{hook}{name}");
                if scripts.contains_key(&h) {
                    t.spec
                        .dep(TaskRef::Local { name: h }, format!("{hook} hook"));
                }
            }
            out.push(t);
        }
        out
    }
}

/// `npm`, `pnpm`, `yarn`, `bun` subcommands that run scripts (and `bun
/// <file>`).
pub struct PackageManager;

impl Recognizer for PackageManager {
    fn id(&self) -> &'static str {
        "package-manager"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["npm", "pnpm", "yarn", "bun", "corepack"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let mut cmd = cmd.clone();
        if cmd.program_name() == Some("corepack") {
            cmd = cmd.shifted(1);
            if cmd.argv.is_empty() {
                return Recognition::Ignored;
            }
        }
        let pm = cmd.program_name().unwrap_or("npm").to_string();
        let words: Vec<String> = cmd.args().map(str::to_string).collect();
        let mut filters: Vec<String> = Vec::new();
        let mut recursive = false;
        let mut dir: Option<String> = None;
        let mut rest: Vec<String> = Vec::new();
        let mut i = 0;
        // Global options before (and mixed with) the subcommand.
        while i < words.len() {
            let w = words[i].as_str();
            i += 1;
            let (k, inline) = match w.split_once('=') {
                Some((k, v)) if k.starts_with('-') => (k, Some(v.to_string())),
                _ => (w, None),
            };
            let mut value = || {
                inline.clone().or_else(|| {
                    i += 1;
                    words.get(i - 1).cloned()
                })
            };
            match k {
                "-F" | "--filter" | "--filter-prod" | "-w" | "--workspace"
                    if !(pm == "pnpm" && k == "-w") =>
                {
                    if let Some(v) = value() {
                        filters.push(v);
                    }
                }
                "-w" | "--workspace-root" if pm == "pnpm" => dir = Some(".root".into()),
                "-r" | "--recursive" | "-ws" | "--workspaces" => recursive = true,
                "-C" | "--dir" | "--prefix" | "--cwd" => dir = value(),
                "--parallel"
                | "--stream"
                | "-s"
                | "--silent"
                | "--if-present"
                | "--sequential"
                | "--no-bail"
                | "--reporter-hide-prefix"
                | "--aggregate-output"
                | "--loglevel"
                | "--frozen-lockfile" => {
                    if k == "--loglevel" {
                        let _ = value();
                    }
                }
                "--" => {
                    rest.extend(words[i..].iter().cloned());
                    break;
                }
                _ if k.starts_with('-') && rest.is_empty() => {}
                _ => rest.push(w.to_string()),
            }
        }
        let sub = rest.first().cloned().unwrap_or_default();
        let (script, extra): (Option<String>, &[String]) = match sub.as_str() {
            "run" | "run-script" | "rum" | "urn" => {
                (rest.get(1).cloned(), rest.get(2..).unwrap_or(&[]))
            }
            "test" | "t" | "tst" | "start" | "stop" | "restart" => {
                let n = match sub.as_str() {
                    "t" | "tst" => "test",
                    s => s,
                };
                (Some(n.to_string()), rest.get(1..).unwrap_or(&[]))
            }
            "workspace" if pm == "yarn" => {
                // yarn workspace <name> [run] <script>
                if let Some(ws) = rest.get(1) {
                    filters.push(ws.clone());
                }
                let mut j = 2;
                if rest.get(j).map(String::as_str) == Some("run") {
                    j += 1;
                }
                (rest.get(j).cloned(), rest.get(j + 1..).unwrap_or(&[]))
            }
            "workspaces" if pm == "yarn" || pm == "npm" => {
                // yarn workspaces run x / yarn workspaces foreach [opts] run x
                recursive = true;
                let pos = rest.iter().position(|w| w == "run");
                match pos {
                    Some(p) => (rest.get(p + 1).cloned(), rest.get(p + 2..).unwrap_or(&[])),
                    None => (None, &[][..]),
                }
            }
            "install" | "i" | "add" | "ci" | "remove" | "rm" | "uninstall" | "update" | "up"
            | "upgrade" | "link" | "unlink" | "prune" | "dedupe" | "rebuild" | "audit"
            | "outdated" | "why" | "list" | "ls" | "info" | "view" | "config" | "set" | "cache"
            | "login" | "logout" | "whoami" | "version" | "-v" | "--version" | "help" | "store"
            | "fetch" | "import" | "patch" | "patch-commit" | "licenses" | "init" | "create"
            | "pack" | "publish" | "deploy" | "dlx" | "exec" | "x" | "env" | "bin" | "root"
            | "pm" | "plugin" | "constraints" | "explain" | "dedup" | "approve-builds"
            | "setup" | "node" | "doctor" | "changeset" => {
                if matches!(sub.as_str(), "install" | "i" | "ci" | "add") {
                    out.config(cx.package_root.join("package.json"));
                }
                if matches!(sub.as_str(), "pack" | "publish" | "deploy") {
                    cx.whole_package(out, &format!("{pm} {sub}"));
                    return Recognition::Partial;
                }
                return Recognition::Ignored;
            }
            "" => return Recognition::Ignored,
            s if pm == "bun" => {
                return bun(&cmd, s, &rest, cx, out);
            }
            s if (pm == "yarn" || pm == "pnpm") && !is_pm_builtin(&pm, s) => {
                (Some(s.to_string()), rest.get(1..).unwrap_or(&[]))
            }
            _ => {
                out.uncertain(UncertaintyKind::UnknownCommand, format!("{pm} {sub}"));
                cx.whole_package(out, &format!("unknown command `{pm} {sub}`"));
                return Recognition::Unknown;
            }
        };
        let Some(script) = script else {
            return Recognition::Ignored;
        };
        if pm == "bun"
            && crate::recognizers::util::is_pathish(&script)
            && !cx.scripts.contains(&script)
        {
            return bun(&cmd, "run", &rest, cx, out);
        }
        let reason = format!("{pm} run");
        // A script that only formats in place, called with arguments that keep it that
        // way (paths, no `--check`): a dependency a producing task drops
        // ([`crate::format`]).
        let formats = |dir: &std::path::Path| {
            let plain = extra.iter().all(|a| {
                let dynamic = cmd.argv.iter().any(|w| w.text == *a && w.dynamic);
                !crate::format::is_check_flag(a) && (!dynamic || a.contains('/'))
            });
            plain
                && crate::format::script_text(dir, &script)
                    .is_some_and(|text| crate::format::format_only(&text, dir, cx))
        };
        if let Some(d) = dir {
            let abs = if d == ".root" {
                cx.repo_root.to_path_buf()
            } else {
                cmd.path(&d)
            };
            let in_place = formats(&abs);
            let target = TaskRef::Package {
                package: cx_rel(cx, &abs),
                name: script,
            };
            if in_place {
                out.dep_in_place(target, reason);
            } else {
                out.dep(target, reason);
            }
        } else if !filters.is_empty() {
            for f in filters {
                out.dep(filter_ref(&f, &script), reason.clone());
            }
        } else if recursive {
            out.dep(TaskRef::All { name: script }, reason);
        } else if cmd.cwd != cx.package_root {
            // `cd ../other && npm run x`
            out.dep(
                TaskRef::Package {
                    package: cx_rel(cx, &cmd.cwd),
                    name: script,
                },
                reason,
            );
        } else if formats(cx.package_root) {
            out.dep_in_place(TaskRef::Local { name: script }, reason);
        } else {
            out.dep(TaskRef::Local { name: script }, reason);
        }
        Recognition::Full
    }
}

/// The package manager of the repository at `root`, from its lockfile (`npm` by
/// default).
fn package_manager_of(root: &std::path::Path) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        "bun"
    } else {
        "npm"
    }
}

/// The repo-relative form of an absolute path.
fn cx_rel(cx: &RecognizeCx<'_>, p: &std::path::Path) -> String {
    match p.strip_prefix(cx.repo_root) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.to_string_lossy().into_owned(),
        Err(_) => p.to_string_lossy().into_owned(),
    }
}

/// A package filter (`web`, `@acme/*`, `./apps/*`, `{apps/web}`, `web...`,
/// `...^web`, `[origin/main]`) → a task reference. Filters that select through
/// the dependency graph or git state over-approximate to every package.
pub fn filter_ref(f: &str, name: &str) -> TaskRef {
    let f = f.trim_matches(['{', '}']);
    if f.contains("...")
        || f.contains('[')
        || f.starts_with('!')
        || f.is_empty()
        || f == "*"
        || f == "**"
    {
        return TaskRef::All {
            name: name.to_string(),
        };
    }
    TaskRef::Package {
        package: f.to_string(),
        name: name.to_string(),
    }
}

fn bun(
    cmd: &Command,
    sub: &str,
    rest: &[String],
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
) -> Recognition {
    out.tool("system", "bun");
    match sub {
        "test" => {
            let args: Vec<&str> = rest
                .iter()
                .skip(1)
                .map(String::as_str)
                .filter(|a| !a.starts_with('-'))
                .collect();
            if args.is_empty() {
                out.membership(
                    cmd.cwd.clone(),
                    "**/*{.test,_test,.spec,_spec}.{js,jsx,ts,tsx,mjs,cjs,mts,cts}",
                    "bun test files",
                );
            }
            for a in args {
                crate::recognizers::util::add_path_input(cmd, a, "bun test files", out);
            }
            for n in ["bunfig.toml", "tsconfig.json"] {
                out.input(cmd.cwd.join(n));
            }
            cx.whole_package(out, "bun test preload and sources");
            Recognition::Full
        }
        "build" => {
            let a = crate::recognizers::util::Args::new(
                cmd,
                1,
                &[
                    "--outdir",
                    "--outfile",
                    "--target",
                    "--format",
                    "--external",
                    "-e",
                    "--sourcemap",
                    "--root",
                ],
            )
            .collect_all();
            for p in crate::recognizers::util::positionals(&a) {
                out.entry(cmd.path(p), EntryLang::Js);
            }
            for o in crate::recognizers::util::opts(&a, &["--outdir", "--outfile"]) {
                out.output(cmd.path(o));
            }
            Recognition::Full
        }
        "run" => {
            // `bun run file.ts` (not a script)
            let skip = cmd.argv.iter().position(|w| w.text == "run").unwrap_or(0);
            node_like(cmd, skip, cx, out)
        }
        s if crate::recognizers::util::is_pathish(s) => node_like(cmd, 0, cx, out),
        s if cx.scripts.iter().any(|x| x == s) => {
            out.dep(
                TaskRef::Local {
                    name: s.to_string(),
                },
                "bun run",
            );
            Recognition::Full
        }
        s => {
            out.uncertain(UncertaintyKind::UnknownCommand, format!("bun {s}"));
            cx.whole_package(out, &format!("unknown command `bun {s}`"));
            Recognition::Unknown
        }
    }
}

/// `npm-run-all`, `run-s`, `run-p` (and `npm-run-all2`): script names and
/// patterns.
pub struct RunAll;

impl Recognizer for RunAll {
    fn id(&self) -> &'static str {
        "npm-run-all"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["npm-run-all", "npm-run-all2", "run-s", "run-p"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "npm-run-all");
        let mut words = cmd.args();
        let mut names = Vec::new();
        while let Some(w) = words.next() {
            match w {
                "--" => break,
                "--npm-path" | "--max-parallel" | "-r" | "--race-delay" => {
                    words.next();
                }
                w if w.starts_with('-') => {}
                w => names.push(w.to_string()),
            }
        }
        let mut complete = true;
        for n in names {
            // Arguments to scripts: `build -- --watch` style "name args".
            let n = n.split_whitespace().next().unwrap_or("").to_string();
            if n.contains('*') {
                let pat = n.replace(':', "/");
                let matched: Vec<&String> = cx
                    .scripts
                    .iter()
                    .filter(|s| crate::glob::matches(&pat, &s.replace(':', "/")))
                    .collect();
                if matched.is_empty() {
                    complete = false;
                }
                for s in matched {
                    out.dep(TaskRef::Local { name: s.clone() }, "npm-run-all pattern");
                }
            } else if !n.is_empty() {
                out.dep(TaskRef::Local { name: n }, "npm-run-all");
            }
        }
        if complete {
            Recognition::Full
        } else {
            Recognition::Partial
        }
    }
}

/// `concurrently "cmd" "npm:x" …`: each argument is a command line.
pub struct Concurrently;

impl Recognizer for Concurrently {
    fn id(&self) -> &'static str {
        "concurrently"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["concurrently", "conc"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "concurrently");
        const VALUE: &[&str] = &[
            "-n",
            "--names",
            "-c",
            "--prefix-colors",
            "-p",
            "--prefix",
            "-l",
            "--prefix-length",
            "-t",
            "--timestamp-format",
            "-s",
            "--success",
            "--restart-tries",
            "--restart-after",
            "-m",
            "--max-processes",
            "--default-input-target",
            "--name-separator",
            "--teardown",
            "--pad-prefix",
        ];
        // Real invocations run a handful of commands. Beyond this many, recognizing
        // each one (shell parse, registry dispatch, binary lookups on the
        // filesystem) costs more than it buys: fall back to the whole package
        // (the registry adds it for the uncertainty).
        const MAX_COMMANDS: usize = 64;
        let words: Vec<&crate::spec::Word> = cmd.arg_words().iter().collect();
        let mut i = 0;
        let mut commands = 0;
        let mut worst = Recognition::Full;
        while i < words.len() {
            let w = words[i];
            i += 1;
            let t = w.text.as_str();
            if t.starts_with('-') {
                if !t.contains('=') && VALUE.contains(&t) {
                    i += 1;
                }
                continue;
            }
            commands += 1;
            if commands > MAX_COMMANDS {
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("concurrently with more than {MAX_COMMANDS} commands"),
                );
                return worse(worst, Recognition::Partial);
            }
            let r = if let Some(s) = t
                .strip_prefix("npm:")
                .or_else(|| t.strip_prefix("pnpm:"))
                .or_else(|| t.strip_prefix("yarn:"))
                .or_else(|| t.strip_prefix("bun:"))
            {
                let name = s.split_whitespace().next().unwrap_or("");
                if name.contains('*') {
                    let pat = name.replace(':', "/");
                    for sc in cx
                        .scripts
                        .iter()
                        .filter(|x| crate::glob::matches(&pat, &x.replace(':', "/")))
                    {
                        out.dep(TaskRef::Local { name: sc.clone() }, "concurrently");
                    }
                } else {
                    out.dep(
                        TaskRef::Local {
                            name: name.to_string(),
                        },
                        "concurrently",
                    );
                }
                Recognition::Full
            } else {
                let parsed = crate::shell::parse(t, &cmd.cwd, cx.env);
                out.uncertainties
                    .extend(parsed.uncertainties.iter().cloned());
                let mut r = if parsed.uncertainties.is_empty() {
                    Recognition::Full
                } else {
                    Recognition::Partial
                };
                for c in &parsed.commands {
                    let rep = registry().recognize(c, cx, out);
                    r = worse(r, rep.recognition);
                }
                r
            };
            worst = worse(worst, r);
        }
        worst
    }
}

/// The less recognized of two results (`Ignored` counts as `Full`).
pub fn worse(a: Recognition, b: Recognition) -> Recognition {
    use Recognition::*;
    match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Partial, _) | (_, Partial) => Partial,
        (Full, _) | (_, Full) => Full,
        _ => Ignored,
    }
}

/// `lerna run x [--scope pkg]`.
pub struct Lerna;

impl Recognizer for Lerna {
    fn id(&self) -> &'static str {
        "lerna"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["lerna"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "lerna");
        let args = crate::recognizers::util::Args::new(
            cmd,
            0,
            &[
                "--scope",
                "--ignore",
                "--concurrency",
                "--since",
                "--stream",
            ],
        )
        .collect_all();
        let pos = crate::recognizers::util::positionals(&args);
        match pos.first() {
            Some(&"run") => {
                let Some(name) = pos.get(1) else {
                    return Recognition::Ignored;
                };
                let scopes = crate::recognizers::util::opts(&args, &["--scope"]);
                if scopes.is_empty() || crate::recognizers::util::opt(&args, &["--since"]).is_some()
                {
                    out.dep(
                        TaskRef::All {
                            name: name.to_string(),
                        },
                        "lerna run",
                    );
                } else {
                    for s in scopes {
                        out.dep(filter_ref(s, name), "lerna run");
                    }
                }
                Recognition::Full
            }
            Some(
                &("version" | "publish" | "bootstrap" | "clean" | "link" | "init" | "changed"
                | "diff" | "info" | "list" | "ls"),
            ) => Recognition::Ignored,
            _ => {
                out.uncertain(UncertaintyKind::UnknownCommand, "lerna");
                cx.whole_package(out, "lerna");
                Recognition::Partial
            }
        }
    }
}
