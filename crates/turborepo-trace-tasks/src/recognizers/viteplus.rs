//! vite-plus, the unified Vite toolchain (`vp`, with the shorthands `vpr` and
//! `vpx`).
//!
//! | Command | What it is |
//! |---|---|
//! | `vp test [run\|watch\|dev\|related\|bench\|list] [filters] [options]` | a vitest run ([`crate::recognizers::testrunners::vitest_run`]): the options are vitest's, the root is the directory `vp` is started in |
//! | `vp run [options] <task>`, `vpr <task>` | the package script `<task>` (`pkg#task`: of that package), with the package selection of `-r`, `-t`, `-w` and `-F`. `-t` selects a package and its workspace dependencies: with it, and when options select in more than one way, the script of every package. A task that is no script is one of a vite config's `run.tasks`, which is not read: the command is partly recognized and its text is scanned for launch words ([`crate::runs`]) |
//! | `vp exec <command>`, `vp dlx <command>`, `vpx <command>` | a wrapper around `<command>` ([`exec_command`]); `vpx [node options] <file>` runs the file with Node. With a package selection or `-c` (a shell line) the text is scanned |
//! | `vp -C <dir> …` | the same, started in `<dir>` |
//!
//! Every other subcommand (`vp build`, `vp lint`, `vp fmt`, `vp check`,
//! `vp pack`, `vp install`, …) is left to the fallbacks for unknown programs.

use std::path::PathBuf;

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::testrunners::vitest_run,
    registry::{Recognition, RecognizeCx, Recognizer, is_script},
    spec::{Command, TaskSpec, Word},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&VitePlus]
}

/// The npm package of `vp`.
const PACKAGE: &str = "vite-plus";

/// A `vp` command line after its own options.
struct Vp {
    /// The index of the subcommand in the words of the command.
    sub: usize,
    /// The directory the subcommand runs in (`-C`).
    cwd: PathBuf,
    /// `-C` names a computed directory.
    computed: bool,
}

/// Reads the options of `vp` itself (`-C <dir>`).
fn parse(cmd: &Command) -> Option<Vp> {
    let mut vp = Vp {
        sub: 1,
        cwd: cmd.cwd.clone(),
        computed: cmd.cwd_computed,
    };
    while cmd.argv.get(vp.sub).is_some_and(|w| w.text == "-C") {
        let dir = cmd.argv.get(vp.sub + 1)?;
        vp.computed |= dir.dynamic;
        vp.cwd = fsx::join_written(&vp.cwd, &dir.text);
        vp.sub += 2;
    }
    cmd.argv.get(vp.sub)?;
    Some(vp)
}

/// Whether `word` names a script file `vpx` runs itself (`vpx ./seed.ts`).
fn is_script_file(word: &str) -> bool {
    let ext = word.rsplit_once('.').map_or("", |(_, e)| e);
    word.starts_with("./")
        || word.starts_with("../")
        || word.starts_with('/')
        || matches!(
            ext,
            "ts" | "mts" | "cts" | "tsx" | "js" | "mjs" | "cjs" | "jsx"
        )
}

/// The command that `vp exec <command>`, `vp dlx <command>` or `vpx <command>`
/// runs in its own directory. `None` when it is another kind of command, when
/// the command runs in other packages (`-r`, `-t`, `-w`, `-F`) or is a shell
/// line (`-c`), and when the directory is computed.
pub fn exec_command(cmd: &Command) -> Option<Command> {
    let vpx = cmd.program_name() == Some("vpx");
    let (mut i, cwd) = if vpx {
        (1, cmd.cwd.clone())
    } else {
        let vp = parse(cmd)?;
        if vp.computed || !matches!(cmd.argv[vp.sub].text.as_str(), "exec" | "dlx") {
            return None;
        }
        (vp.sub + 1, vp.cwd)
    };
    // `vpx [options] [node options] <script>`: an option that is not one of
    // vpx's own starts Node's, which Node's command line reads (`--import
    // ./setup.ts ./main.ts`: the script is `main.ts`).
    let mut node_options = false;
    while let Some(w) = cmd.argv.get(i) {
        match w.text.as_str() {
            "--" => {
                i += 1;
                break;
            }
            "-c" | "--shell-mode" => return None,
            "-p" | "--package" | "--tsconfig" => i += 2,
            a if vpx && (a.starts_with("--package=") || a.starts_with("--tsconfig=")) => i += 1,
            "-s" | "--silent" if vpx => i += 1,
            a if vpx && a.starts_with('-') => {
                node_options = true;
                break;
            }
            "-r" | "--recursive" | "-t" | "--transitive" | "-w" | "--workspace-root" | "-F"
            | "--filter" => return None,
            a if a.starts_with("--filter=") => return None,
            "--resume-from" => i += 2,
            a if a.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let first = cmd.argv.get(i)?;
    if first.dynamic {
        return None;
    }
    let mut inner = cmd.shifted(i);
    inner.cwd = cwd;
    if vpx && (node_options || is_script_file(&first.text)) {
        // `vpx <file>` runs a script as `tsx` does: Node with a loader.
        inner.argv.insert(0, Word::lit("node"));
    }
    Some(inner)
}

/// `vp test`, `vp run`, `vpr`, and the `vp exec` / `vpx` forms that
/// [`exec_command`] leaves.
pub struct VitePlus;

impl Recognizer for VitePlus {
    fn id(&self) -> &'static str {
        "vite-plus"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["vp", "vpr", "vpx"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        match cmd.program_name() {
            Some("vpr" | "vpx") => true,
            Some("vp") => parse(cmd).is_some_and(|vp| {
                matches!(
                    cmd.argv[vp.sub].text.as_str(),
                    "test" | "run" | "exec" | "dlx"
                )
            }),
            _ => false,
        }
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", PACKAGE);
        let prog = cmd.program_name().unwrap_or("vp");
        let (vp, sub) = match prog {
            "vp" => match parse(cmd) {
                Some(vp) => {
                    let sub = cmd.argv[vp.sub].text.clone();
                    (vp, sub)
                }
                None => return Recognition::Ignored,
            },
            _ => {
                let vp = Vp {
                    sub: 0,
                    cwd: cmd.cwd.clone(),
                    computed: cmd.cwd_computed,
                };
                (vp, if prog == "vpr" { "run" } else { "exec" }.to_string())
            }
        };
        if vp.computed && !cmd.cwd_computed {
            out.uncertain(
                UncertaintyKind::ShellUnsupported,
                format!("`{prog} -C` with a computed directory"),
            );
            return Recognition::Partial;
        }
        match sub.as_str() {
            "test" => {
                // vp starts the vitest it depends on, in the directory it is started in.
                out.tool("npm", "vitest");
                let mut run = cmd.shifted(vp.sub);
                run.cwd = vp.cwd;
                vitest_run(&run, cx, out)
            }
            "run" => run_task(cmd, vp, cx, out),
            _ => {
                out.uncertain(
                    UncertaintyKind::UnknownCommand,
                    format!("`{prog} {sub}` in other packages or in a shell"),
                );
                Recognition::Partial
            }
        }
    }
}

/// `vp run [options] <task> [args]`: the package script `<task>`, as the
/// package manager's `run` (`vp.sub` is the index of `run`, or of `vpr`).
fn run_task(cmd: &Command, vp: Vp, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
    // The selections: every package (`-r`; `-t`, a package and its workspace
    // dependencies, which is any package), the root (`-w`), and filters (`-F`
    // and `pkg#task`).
    let mut wide = false;
    let mut root = false;
    let mut filters: Vec<Word> = Vec::new();
    let mut task: Option<&Word> = None;
    let mut i = vp.sub + 1;
    while let Some(w) = cmd.argv.get(i) {
        i += 1;
        match w.text.as_str() {
            "-r" | "--recursive" | "-t" | "--transitive" => wide = true,
            "-w" | "--workspace-root" => root = true,
            "-F" | "--filter" => {
                filters.extend(cmd.argv.get(i).cloned());
                i += 1;
            }
            a if a.starts_with("--filter=") => filters.push(Word {
                text: a["--filter=".len()..].to_string(),
                ..w.clone()
            }),
            "--concurrency-limit" | "--log" => i += 1,
            a if a.starts_with('-') => {}
            _ => {
                task = Some(w);
                break;
            }
        }
    }
    let Some(task) = task else {
        // The task selector, `--last-details`.
        return Recognition::Ignored;
    };
    let name = match task.text.rsplit_once('#') {
        Some((package, name)) if !package.is_empty() => {
            filters.push(Word::lit(package));
            name
        }
        _ => task.text.as_str(),
    };
    // The package manager's command line takes one kind of selection: a
    // combination (`-t app#test`: `app` and its dependencies; `-w -F app`) is
    // the script of every package.
    let mut pm = vec![Word::lit("pnpm")];
    if wide || (root && !filters.is_empty()) {
        pm.push(Word::lit("-r"));
    } else if root {
        pm.push(Word::lit("-w"));
    } else {
        for f in filters {
            pm.push(Word::lit("--filter"));
            pm.push(f);
        }
    }
    let mut run = cmd.clone();
    run.cwd = vp.cwd;
    if pm.len() == 1 && !task.dynamic && !is_script(cx, &run, name) {
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("`vp run {name}`: no package script, a task of a vite config"),
        );
        return Recognition::Partial;
    }
    pm.push(Word::lit("run"));
    pm.push(Word {
        text: name.to_string(),
        ..task.clone()
    });
    pm.extend(cmd.argv[i..].iter().cloned());
    run.argv = pm;
    let before = out.task_deps.len();
    let recognition = crate::sources::npm::PackageManager.recognize(&run, cx, out);
    for dep in &mut out.task_deps[before..] {
        dep.reason = "vp run".into();
    }
    recognition
}
