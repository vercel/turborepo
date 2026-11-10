//! The tool registry: [`Recognizer`]s map one parsed [`Command`] to additions
//! to a [`TaskSpec`].
//!
//! # Contract
//!
//! - A recognizer **never executes** anything. It reads files only through the
//!   context ([`RecognizeCx::read`], [`RecognizeCx::is_file`], ... which route
//!   through [`turborepo_trace_frameworks::fsx`], i.e. the tracer's shared
//!   `DirCache` when the driver runs inside [`fsx::with_fs`]), so config
//!   discovery probes are recorded.
//! - It only *adds* to the spec. Paths are absolute ([`Command::path`] resolves
//!   against the command's cwd).
//! - Any uncertainty a recognizer adds automatically gets the package directory
//!   as a membership ([`Registry::recognize`] enforces it), so a task's
//!   uncertainty is always covered by a conservative input set.
//! - It returns a [`Recognition`] for the coverage report:
//!   [`Recognition::Full`] when every input of the command is modelled,
//!   [`Recognition::Partial`] when something was not (and an uncertainty or a
//!   conservative membership was added), [`Recognition::Ignored`] for commands
//!   with no file effect (`echo`, `rimraf` of an output dir). A recognizer
//!   whose [`Recognizer::matches`] returned true but that cannot make sense of
//!   the arguments should still return `Partial` and add the package directory
//!   as a membership ([`RecognizeCx::whole_package`]).
//! - Wrappers (`npx`, `pnpm exec`, `yarn`, `bunx`, `cross-env`, `dotenv --`,
//!   `env`, `time`, `nice`, `node_modules/.bin/`) are stripped by
//!   [`Registry::recognize`] before matching, so recognizers see the real
//!   program in `argv[0]`.
//!
//! # Registration
//!
//! [`registry`] is a static list: the recognizers of [`crate::recognizers`]
//! (run, js, testrunners, fs, codegen) and those exported by task-source
//! modules ([`crate::sources::recognizers`], e.g. `make -C` and `docker build`
//! from the make/docker sources). Earlier recognizers win; owners add theirs in
//! their own module's `recognizers()` without touching this file.
//!
//! # Example
//!
//! ```
//! use turborepo_trace_tasks::registry::{Recognition, RecognizeCx, Recognizer};
//! use turborepo_trace_tasks::spec::{Command, EntryLang, TaskSpec};
//!
//! struct Gen;
//! impl Recognizer for Gen {
//!     fn id(&self) -> &'static str { "my-gen" }
//!     fn programs(&self) -> &'static [&'static str] { &["my-gen"] }
//!     fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
//!         for a in cmd.args() {
//!             out.input(cmd.path(a));
//!         }
//!         out.tool("npm", "my-gen");
//!         let _ = cx;
//!         Recognition::Full
//!     }
//! }
//! ```

use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Serialize;
use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::spec::{Command, EntryLang, OutputKind, TaskSpec};

/// Recognizers whose commands only read the files of their memberships
/// ([`crate::spec::Membership::data`]): linters and formatters parse the files
/// they are pointed at; what they run is their config and plugins (entries).
/// Everything else is decided where the membership is made (`tsc` by its emit
/// options, turbo's declared inputs) or stays the default: the command may run
/// or compile the files.
const READ_ONLY: &[&str] = &["lint"];

/// How well a command was understood (for a coverage report).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recognition {
    /// Every input and output of the command is modelled.
    Full,
    /// Recognized, but some part is not modelled (an uncertainty was added).
    Partial,
    /// No file effect worth recording (`echo`, `sleep`, `rimraf dist`).
    Ignored,
    /// No recognizer matched: the package directory became a membership.
    Unknown,
}

/// What a recognizer sees besides the command.
pub struct RecognizeCx<'a> {
    /// Repository root (absolute).
    pub repo_root: &'a Path,
    /// Root of the package the task belongs to (absolute; the `package.json`,
    /// Makefile or manifest directory).
    pub package_root: &'a Path,
    /// Known environment (`VAR=x` prefixes of the command already applied).
    pub env: &'a BTreeMap<String, String>,
    /// The task's name (`build`), when the command comes from a named task.
    pub task_name: Option<&'a str>,
    /// Names of the scripts of the package (so `pnpm x` can tell a script from
    /// a binary).
    pub scripts: &'a [String],
}

impl RecognizeCx<'_> {
    /// Reads a file (through the shared cache when active).
    pub fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        fsx::read(p)
    }

    /// Reads a UTF-8 file.
    pub fn read_to_string(&self, p: &Path) -> io::Result<String> {
        fsx::read_to_string(p)
    }

    /// Whether `p` is a file.
    pub fn is_file(&self, p: &Path) -> bool {
        fsx::is_file(p)
    }

    /// Whether `p` is a directory.
    pub fn is_dir(&self, p: &Path) -> bool {
        fsx::is_dir(p)
    }

    /// The first existing `dir/name` among `names`.
    pub fn first_existing(&self, dir: &Path, names: &[&str]) -> Option<PathBuf> {
        fsx::first_existing(dir, names)
    }

    /// Adds the conservative fallback: every file in the package directory is
    /// an input.
    pub fn whole_package(&self, out: &mut TaskSpec, reason: &str) {
        out.membership(self.package_root, "**", reason);
    }
}

/// Recognizes one kind of command.
pub trait Recognizer: Send + Sync {
    /// Stable id (`tsc`, `prisma`), shown in task listings.
    fn id(&self) -> &'static str;

    /// Program base names this recognizer handles (`["tsc"]`). Used by the
    /// default [`Recognizer::matches`] and for dispatch indexing.
    fn programs(&self) -> &'static [&'static str];

    /// Whether this recognizer handles `cmd` (after wrapper stripping). The
    /// default matches [`Command::program_name`] against
    /// [`Recognizer::programs`]. Override to match on argv shape (`go
    /// generate`, `docker build`, `python -m grpc_tools.protoc`).
    fn matches(&self, cmd: &Command) -> bool {
        cmd.program_name()
            .is_some_and(|p| self.programs().contains(&p))
    }

    /// Adds what `cmd` reads and writes to `out`.
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition;
}

/// The result of recognizing one command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandReport {
    /// The command text.
    pub text: String,
    /// The real program after wrapper stripping (`None` for a command with no
    /// words).
    pub program: Option<String>,
    /// The recognizer that handled it.
    pub recognizer: Option<&'static str>,
    /// How well.
    pub recognition: Recognition,
}

/// The ordered recognizer list with dispatch.
pub struct Registry {
    recognizers: Vec<&'static dyn Recognizer>,
    /// The first `generators` recognizers are the code generators
    /// ([`crate::recognizers::codegen::recognizers`], specs included): their
    /// outputs are [`OutputKind::Generated`].
    generators: usize,
}

/// The compiled-in registry.
pub fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| {
        let mut recognizers = crate::recognizers::recognizers();
        // `recognizers()` starts with the code generators.
        let generators = crate::recognizers::codegen::recognizers().len();
        debug_assert!(
            crate::recognizers::codegen::recognizers()
                .iter()
                .zip(&recognizers)
                .all(|(a, b)| a.id() == b.id())
        );
        recognizers.extend(crate::sources::recognizers());
        // Catch-alls last.
        recognizers.extend(crate::recognizers::fallback_recognizers());
        Registry {
            recognizers,
            generators,
        }
    })
}

/// Programs that wrap another command (some only with a `run`/`exec`
/// subcommand); the handling of their options is in [`strip_wrappers`].
pub const WRAPPERS: &[&str] = &[
    "npx",
    "bunx",
    "pnpx",
    "cross-env",
    "cross-env-shell",
    "env",
    "time",
    "nice",
    "nohup",
    "dotenv",
    "dotenvx",
    "env-cmd",
    "sudo",
    "exec",
    "command",
    "doppler",
    "op",
    "infisical",
    "uv",
    "uvx",
    "poetry",
    "pipenv",
    "pdm",
    "hatch",
    "rye",
    "pipx",
];

/// Whether `name` is a script the command can mean: one of the task's own
/// package, or one of the package at the command's directory (after `cd
/// other/pkg`, and for tasks that are not package scripts themselves, such as
/// the steps of a CI job, which have no script list).
fn is_script(cx: &RecognizeCx<'_>, cmd: &Command, name: &str) -> bool {
    if cx.scripts.iter().any(|s| s == name) {
        return true;
    }
    if cmd.cwd == cx.package_root && !cx.scripts.is_empty() {
        return false;
    }
    cx.read(&cmd.cwd.join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v.get("scripts").and_then(|s| s.get(name)).is_some())
}

/// Strips wrapper programs (`npx tsc` → `tsc`, `pnpm exec`, `yarn tsc` when
/// `tsc` is not a script, `cross-env A=b cmd`, `dotenv -e .env -- cmd`,
/// `node_modules/.bin/tsc`). Env files named by dotenv wrappers are added as
/// inputs of `out`; the wrapper itself is added as a toolchain package.
pub fn strip_wrappers(cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Command {
    let mut c = cmd.clone();
    for _ in 0..8 {
        let Some(name) = c.program_name().map(str::to_string) else {
            return c;
        };
        let args: Vec<String> = c.args().map(str::to_string).collect();
        let next = match name.as_str() {
            "npx" | "bunx" | "pnpx" => {
                // Options before the package: `-y`, `--yes`, `-p pkg`, `--package=pkg`.
                let mut i = 1;
                while let Some(a) = c.argv.get(i).map(|w| w.text.as_str()) {
                    if a == "-p" || a == "--package" {
                        i += 2;
                    } else if a.starts_with('-') {
                        i += 1;
                    } else {
                        break;
                    }
                }
                i
            }
            "pnpm" | "yarn" | "bun" => match args.first().map(String::as_str) {
                Some("exec" | "dlx" | "x") => {
                    let mut i = 2;
                    while c.argv.get(i).is_some_and(|w| w.text.starts_with('-')) {
                        i += 1;
                    }
                    i
                }
                // `yarn tsc`, `pnpm tsc`: a binary when it is not a script or builtin.
                Some(a)
                    if !a.starts_with('-')
                        && !(name == "bun" && crate::recognizers::util::is_pathish(a))
                        && !is_pm_builtin(&name, a)
                        && !is_script(cx, &c, a) =>
                {
                    1
                }
                _ => return c,
            },
            "cross-env" | "cross-env-shell" | "env" => {
                let mut i = 1;
                while let Some(w) = c.argv.get(i) {
                    if let Some((k, v)) = w.text.split_once('=')
                        && !k.is_empty()
                        && !k.contains('/')
                        && !k.starts_with('-')
                    {
                        c.assigns.push((k.to_string(), crate::spec::Word::lit(v)));
                        i += 1;
                        continue;
                    }
                    if name == "env" && w.text.starts_with('-') {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i
            }
            "dotenv" | "dotenvx" | "env-cmd" => {
                // Up to `--`, or the first non-option word.
                let mut i = 1;
                let mut files = Vec::new();
                while let Some(w) = c.argv.get(i) {
                    let a = w.text.as_str();
                    if a == "--" {
                        i += 1;
                        break;
                    }
                    if a == "run" && name == "dotenvx" {
                        i += 1;
                        continue;
                    }
                    if matches!(a, "-e" | "-f" | "--file" | "-c" | "--env-file") {
                        if let Some(f) = c.argv.get(i + 1) {
                            files.push(f.text.clone());
                        }
                        i += 2;
                        continue;
                    }
                    if let Some(f) = a
                        .strip_prefix("--file=")
                        .or_else(|| a.strip_prefix("--env-file="))
                    {
                        files.push(f.to_string());
                        i += 1;
                        continue;
                    }
                    if a.starts_with('-') {
                        i += 1;
                        continue;
                    }
                    break;
                }
                if files.is_empty() {
                    out.input(c.path(".env"));
                }
                for f in files {
                    out.input(c.path(&f));
                }
                i
            }
            "time" | "nice" | "nohup" | "sudo" | "exec" | "command" => {
                let mut i = 1;
                while c.argv.get(i).is_some_and(|w| w.text.starts_with('-')) {
                    i += 1;
                }
                i
            }
            "uv" | "poetry" | "pipenv" | "pdm" | "hatch" | "rye" | "uvx" | "pipx" => {
                // `uv run [opts] cmd`, `poetry run cmd`, `uvx tool`, `pipx run tool`.
                let start = match (name.as_str(), args.first().map(String::as_str)) {
                    ("uvx", _) => 1,
                    (_, Some("run")) => 2,
                    ("hatch", Some("env")) if args.get(1).is_some_and(|a| a == "run") => 3,
                    _ => return c,
                };
                let mut i = start;
                while let Some(w) = c.argv.get(i).map(|w| w.text.as_str()) {
                    if w == "--" {
                        i += 1;
                        break;
                    }
                    if !w.starts_with('-') {
                        break;
                    }
                    // Options with a value.
                    if matches!(
                        w,
                        "--with"
                            | "--python"
                            | "-p"
                            | "--env"
                            | "-e"
                            | "--group"
                            | "--extra"
                            | "--package"
                            | "--directory"
                            | "--project"
                            | "--env-file"
                    ) {
                        if w == "--env-file"
                            && let Some(f) = c.argv.get(i + 1)
                        {
                            out.input(c.path(&f.text));
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.tool("pypi", name.as_str());
                for n in [
                    "pyproject.toml",
                    "uv.lock",
                    "poetry.lock",
                    "Pipfile.lock",
                    "pdm.lock",
                ] {
                    if let Some(p) =
                        crate::recognizers::util::find_up(&c.cwd, n, Some(cx.repo_root))
                    {
                        out.config(p);
                    }
                }
                i
            }
            "doppler" | "op" | "infisical" => {
                // `doppler run -- cmd`, `op run -- cmd`, `infisical run -- cmd`.
                match c.argv.iter().position(|w| w.text == "--") {
                    Some(p) if args.first().is_some_and(|a| a == "run") => {
                        out.uncertain(
                            UncertaintyKind::EnvDependent,
                            format!("secrets injected by `{name}`"),
                        );
                        p + 1
                    }
                    _ => return c,
                }
            }
            _ => return c,
        };
        if matches!(
            name.as_str(),
            "cross-env" | "cross-env-shell" | "dotenv" | "dotenvx" | "env-cmd"
        ) {
            out.tool(
                "npm",
                if name == "dotenv" {
                    "dotenv-cli"
                } else {
                    name.as_str()
                },
            );
        }
        if next >= c.argv.len() {
            return c;
        }
        c = c.shifted(next);
    }
    c
}

/// Subcommands of npm, pnpm, yarn and bun that are not binaries.
pub fn is_pm_builtin(pm: &str, sub: &str) -> bool {
    const COMMON: &[&str] = &[
        "run",
        "install",
        "i",
        "add",
        "remove",
        "rm",
        "uninstall",
        "test",
        "t",
        "start",
        "exec",
        "dlx",
        "x",
        "create",
        "init",
        "publish",
        "pack",
        "link",
        "unlink",
        "why",
        "list",
        "ls",
        "outdated",
        "update",
        "up",
        "upgrade",
        "audit",
        "config",
        "set",
        "info",
        "version",
        "workspace",
        "workspaces",
        "-r",
        "--recursive",
        "-F",
        "--filter",
        "-C",
        "--dir",
        "--cwd",
        "--prefix",
        "-w",
        "--workspace",
        "ci",
        "rebuild",
        "prune",
        "dedupe",
        "store",
        "import",
        "patch",
        "patch-commit",
        "node",
        "env",
        "help",
        "login",
        "logout",
        "cache",
        "global",
        "bin",
        "root",
        "licenses",
        "plugin",
        "constraints",
        "dedup",
        "fetch",
        "deploy",
        "restart",
        "stop",
        "explain",
        "npm",
        "pm",
        "-s",
        "--silent",
    ];
    // `bun build` is the bundler; `pnpm build`/`yarn build` run the `build` script.
    (pm == "bun" && sub == "build") || COMMON.contains(&sub)
}

// ---- Unknown-program hook
// -------------------------

/// Runs before the script-file and unknown-command fallbacks: resolves the
/// program as an npm package binary or a Python console script
/// ([`crate::recognizers::bins`]). Applies the same contract as
/// [`Registry::recognize`] does for recognizers (every uncertainty gets the
/// package membership; computed arguments fall back to the package directory).
fn unknown_program_hook(
    cmd: &Command,
    real: &Command,
    cx: &RecognizeCx<'_>,
    out: &mut TaskSpec,
    prog: &str,
) -> Option<CommandReport> {
    let uncertain_before = out.uncertainties.len();
    let r = crate::recognizers::bins::resolve_unknown(real, cx, out)?;
    let mut recognition = r.recognition;
    if out.uncertainties.len() > uncertain_before {
        cx.whole_package(out, &format!("`{prog}` not fully recognized"));
    }
    if recognition != Recognition::Ignored && real.argv.iter().any(|w| w.dynamic) {
        cx.whole_package(out, &format!("computed arguments to `{prog}`"));
        if recognition == Recognition::Full {
            recognition = Recognition::Partial;
        }
    }
    Some(CommandReport {
        text: cmd.text.clone(),
        program: Some(prog.to_string()),
        recognizer: Some(r.recognizer),
        recognition,
    })
}

// ---- End of unknown-program hook
// ----------------------------------------------------------

impl Registry {
    /// The registered recognizers, in priority order.
    pub fn recognizers(&self) -> &[&'static dyn Recognizer] {
        &self.recognizers
    }

    /// The recognizer for `cmd` (already stripped of wrappers), if any.
    pub fn find(&self, cmd: &Command) -> Option<&'static dyn Recognizer> {
        self.find_index(cmd).map(|i| self.recognizers[i])
    }

    fn find_index(&self, cmd: &Command) -> Option<usize> {
        self.recognizers.iter().position(|r| r.matches(cmd))
    }

    /// Recognizes one command: strips wrappers, dispatches, and for an unknown
    /// program adds the package directory as a membership plus an
    /// [`UncertaintyKind::UnknownCommand`] naming it.
    pub fn recognize(
        &self,
        cmd: &Command,
        cx: &RecognizeCx<'_>,
        out: &mut TaskSpec,
    ) -> CommandReport {
        // The tests the command runs ([`crate::runs`]) are derived from what it adds.
        let mark = crate::runs::mark(out);
        let rep = self.recognize_command(cmd, cx, out);
        crate::runs::observe(cmd, &rep, cx, mark, out);
        rep
    }

    fn recognize_command(
        &self,
        cmd: &Command,
        cx: &RecognizeCx<'_>,
        out: &mut TaskSpec,
    ) -> CommandReport {
        let real = strip_wrappers(cmd, cx, out);
        // Absolute paths the command writes out: what they name does not depend on
        // where the checkout is ([`TaskSpec::written`]).
        let words = real
            .argv
            .iter()
            .chain(real.redirects.iter().map(|r| &r.target));
        for w in words.filter(|w| w.is_static() && !w.glob) {
            let text = match w.text.split_once("=/") {
                Some((flag, rest)) if flag.starts_with('-') => {
                    &w.text[w.text.len() - rest.len() - 1..]
                }
                _ => w.text.as_str(),
            };
            if real.writes(text) {
                let p = fsx::normalize(Path::new(text));
                // Not a path below the root of this checkout: a runner's own location is
                // expanded into the text of its commands (`{toxinidir}/x`,
                // `{{justfile_directory()}}/x`, `${{ github.workspace }}/x`), and that path
                // is the tool's, to be looked up where it is.
                if !p.starts_with(cx.repo_root) && !out.written.contains(&p) {
                    out.written.push(p);
                }
            }
        }
        let mut env = cx.env.clone();
        for (k, v) in &real.assigns {
            if v.is_static() {
                env.insert(k.clone(), v.text.clone());
            }
        }
        let cx2 = RecognizeCx { env: &env, ..*cx };
        let found = self.find_index(&real);
        let generator = found.is_some_and(|i| i < self.generators);
        // Redirections: inputs and outputs, whatever the program. A generator's stdout
        // is a generated file (committed or not). Any other program's
        // truncating redirection (`>`) can't be a read-modify-write of its
        // target (the shell truncates it before the command runs), so an
        // existing target is a committed generated file (`node gen.js >
        // src/generated/client.js`) and a missing one an artifact. An append
        // (`>>`) keeps the old contents: a generic write (in-place for an existing
        // file).
        for r in &real.redirects {
            if r.is_null() || r.target.dynamic {
                continue;
            }
            let p = real.path(&r.target.text);
            if r.is_input() {
                out.input(p);
            } else if r.op != "<<<" {
                if generator {
                    out.output_of_kind(p, OutputKind::Generated);
                } else if r.op.contains(">>") {
                    out.write(p, cx.repo_root);
                } else {
                    let kind = match crate::spec::write_kind(&p, cx.repo_root) {
                        OutputKind::InPlace => OutputKind::Generated,
                        k => k,
                    };
                    out.output_of_kind(p, kind);
                }
            }
        }
        let program = real.program_name().map(str::to_string);
        let Some(prog) = program.clone() else {
            return CommandReport {
                text: cmd.text.clone(),
                program,
                recognizer: None,
                recognition: Recognition::Ignored,
            };
        };
        if let Some(r) = found.map(|i| self.recognizers[i]) {
            let uncertain_before = out.uncertainties.len();
            let outputs_before = out.outputs.len();
            let memberships_before = out.memberships.len();
            let mut recognition = r.recognize(&real, &cx2, out);
            if READ_ONLY.contains(&r.id()) {
                for m in out.memberships.iter_mut().skip(memberships_before) {
                    m.data = true;
                }
            }
            if generator {
                // Code generator outputs link back, including committed generated files.
                out.classify_outputs_since(outputs_before, OutputKind::Generated);
            }
            // Contract: every uncertainty comes with the conservative fallback, so the
            // graph never has to treat a task's uncertainty as "anything may be
            // affected".
            if out.uncertainties.len() > uncertain_before {
                cx.whole_package(out, &format!("`{prog}` not fully recognized"));
            }
            // Computed arguments may name any file: fall back to the package directory.
            if recognition != Recognition::Ignored && real.argv.iter().any(|w| w.dynamic) {
                cx.whole_package(out, &format!("computed arguments to `{prog}`"));
                if recognition == Recognition::Full {
                    recognition = Recognition::Partial;
                }
            }
            return CommandReport {
                text: cmd.text.clone(),
                program,
                recognizer: Some(r.id()),
                recognition,
            };
        }
        if let Some(rep) = unknown_program_hook(cmd, &real, &cx2, out, &prog) {
            return rep;
        }
        // A program whose path is computed (`"$ROOT/scripts/build.sh"`, `./bin/*`):
        // some file runs, and the word does not say which.
        if real.argv[0].text.contains('/') && (real.argv[0].dynamic || real.argv[0].glob) {
            out.uncertain(
                UncertaintyKind::ComputedSpecifier,
                format!("runs a computed script `{}`", real.argv[0].text),
            );
            cx.whole_package(out, &format!("computed script `{prog}`"));
            return CommandReport {
                text: cmd.text.clone(),
                program,
                recognizer: None,
                recognition: Recognition::Unknown,
            };
        }
        // A program that is itself a script file: `./scripts/build.sh`,
        // `scripts/gen.mjs`.
        if real.argv[0].text.contains('/') {
            let p = real.path(&real.argv[0].text);
            let lang = EntryLang::of(&p);
            out.entry(p.clone(), lang);
            if lang == EntryLang::Auto {
                // A shell script or binary: its contents are an input, its effects unknown.
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("script `{}`", real.argv[0].text),
                );
                cx.whole_package(out, &format!("unknown command `{prog}`"));
                return CommandReport {
                    text: cmd.text.clone(),
                    program,
                    recognizer: Some("script-file"),
                    recognition: Recognition::Partial,
                };
            }
            return CommandReport {
                text: cmd.text.clone(),
                program,
                recognizer: Some("script-file"),
                recognition: Recognition::Full,
            };
        }
        out.uncertain(UncertaintyKind::UnknownCommand, prog.clone());
        cx.whole_package(out, &format!("unknown command `{prog}`"));
        CommandReport {
            text: cmd.text.clone(),
            program,
            recognizer: None,
            recognition: Recognition::Unknown,
        }
    }
}
