//! File utilities: `cp`, `mv`, `cat`, `rsync`, `ln`, `tar`, `touch`, `mkdir`,
//! `rm`, and no-op builtins (`echo`, `sleep`, `test`). Registered last
//! (catch-alls).
//!
//! Their writes are generic ([`TaskSpec::write`]): a destination that is an
//! existing source file is an in-place edit (`mv package.json.tmp
//! package.json`), which is recorded but does not make the task a producer;
//! anything else is an artifact.

use crate::{
    recognizers::util::{Arg, Args, add_path_input, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&Copy, &Cat, &Touch, &Tar, &Ignore]
}

/// `cp`, `mv`, `rsync`, `ln`, `cpy`, `copyfiles`, `shx cp`, `ncp`, `cpx`:
/// sources are inputs, the last argument (or `-t dir`) is an output.
pub struct Copy;

impl Recognizer for Copy {
    fn id(&self) -> &'static str {
        "cp"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "cp",
            "mv",
            "rsync",
            "ln",
            "cpy",
            "copyfiles",
            "ncp",
            "cpx",
            "copy",
            "install",
            // Only `shx cp|mv|ln` (see `matches`).
            "shx",
        ]
    }
    fn matches(&self, cmd: &Command) -> bool {
        match cmd.program_name() {
            Some("shx") => matches!(cmd.args().next(), Some("cp" | "mv" | "ln")),
            Some(p) => self.programs().contains(&p),
            None => false,
        }
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let (cmd, prog) = shx(cmd);
        let with_value: &[&str] = match prog.as_str() {
            "rsync" => &[
                "-e",
                "--exclude",
                "--include",
                "--filter",
                "--exclude-from",
                "--chmod",
            ],
            "cpy" => &["--cwd", "--rename"],
            "copyfiles" => &["-u", "--up", "-e", "--exclude", "-f"],
            "install" => &["-m", "-o", "-g", "-t"],
            _ => &["-t", "--target-directory"],
        };
        let args = Args::new(&cmd, 0, with_value).collect_all();
        let mut pos = positionals(&args);
        let target = args.iter().find_map(|a| match a {
            Arg::Opt("-t" | "--target-directory", v) => *v,
            _ => None,
        });
        let dest = match target {
            Some(t) => Some(t),
            None if pos.len() >= 2 => pos.pop(),
            None => None,
        };
        let Some(dest) = dest else {
            return Recognition::Partial;
        };
        let mut partial = false;
        for s in &pos {
            if s.contains("://") || (prog == "rsync" && s.contains(':')) {
                partial = true;
                continue;
            }
            add_path_input(&cmd, s, &format!("{prog} source"), out);
        }
        if !dest.contains("://") {
            out.write(cmd.path(dest), cx.repo_root);
        }
        if prog == "mv" {
            // Moving deletes the source: it is an output too.
            for s in &pos {
                out.write(cmd.path(s), cx.repo_root);
            }
        }
        if cmd.argv.iter().any(|w| w.dynamic) || partial {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// `cat`, `head`, `tail`, `wc`, `sort`, `grep -f`, `diff`, `cmp`, `jq`,
/// `sha256sum`: every file argument is an input (output only through
/// redirections).
pub struct Cat;

impl Recognizer for Cat {
    fn id(&self) -> &'static str {
        "cat"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "cat",
            "head",
            "tail",
            "wc",
            "sort",
            "uniq",
            "diff",
            "cmp",
            "sha256sum",
            "shasum",
            "md5sum",
            "base64",
            "tac",
        ]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let args = Args::new(cmd, 0, &["-n", "-c"]).collect_all();
        for p in positionals(&args) {
            if p != "-" {
                add_path_input(cmd, p, "read", out);
            }
        }
        if cmd.argv.iter().any(|w| w.dynamic) {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// `touch`, `mkdir`: outputs.
pub struct Touch;

impl Recognizer for Touch {
    fn id(&self) -> &'static str {
        "touch"
    }
    fn programs(&self) -> &'static [&'static str] {
        // `shx` only with `touch|mkdir` (see `matches`).
        &["touch", "mkdir", "mkdirp", "make-dir", "shx"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        match cmd.program_name() {
            Some("shx") => matches!(cmd.args().next(), Some("touch" | "mkdir")),
            Some(p) => self.programs().contains(&p),
            None => false,
        }
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let (cmd, prog) = shx(cmd);
        let args = Args::new(&cmd, 0, &["-m", "-d", "-r", "-t"]).collect_all();
        if prog == "touch" {
            for p in positionals(&args) {
                out.write(cmd.path(p), cx.repo_root);
            }
        }
        Recognition::Ignored
    }
}

/// `tar`, `zip`, `unzip`, `gzip`: archive and members.
pub struct Tar;

impl Recognizer for Tar {
    fn id(&self) -> &'static str {
        "tar"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["tar", "zip", "unzip", "gzip", "gunzip"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("");
        let words: Vec<&str> = cmd.args().collect();
        match prog {
            "tar" => {
                let mode = words.first().copied().unwrap_or("");
                let extract = mode.contains('x');
                let args = Args::new(cmd, 0, &["-f", "--file", "-C", "--directory", "--exclude"])
                    .collect_all();
                let mut file = crate::recognizers::util::opt(&args, &["-f", "--file"]);
                let mut pos = positionals(&args);
                if file.is_none() && !mode.starts_with('-') && mode.contains('f') && pos.len() >= 2
                {
                    // `tar czf out.tgz dir`
                    pos.remove(0);
                    file = Some(pos.remove(0));
                } else if !mode.starts_with('-') && !pos.is_empty() {
                    pos.remove(0);
                }
                let base = match crate::recognizers::util::opt(&args, &["-C", "--directory"]) {
                    Some(d) => cmd.path(d),
                    None => cmd.cwd.clone(),
                };
                let sub = Command {
                    cwd: base.clone(),
                    ..cmd.clone()
                };
                if extract {
                    if let Some(f) = file {
                        out.input(cmd.path(f));
                    }
                    out.write(base, cx.repo_root);
                } else {
                    if let Some(f) = file {
                        out.write(cmd.path(f), cx.repo_root);
                    }
                    for p in pos {
                        add_path_input(&sub, p, "archived", out);
                    }
                }
                Recognition::Full
            }
            "zip" => {
                let args = Args::new(cmd, 0, &["-x", "-i"]).collect_all();
                let mut pos = positionals(&args).into_iter();
                if let Some(a) = pos.next() {
                    out.write(cmd.path(a), cx.repo_root);
                }
                for p in pos {
                    add_path_input(cmd, p, "archived", out);
                }
                Recognition::Full
            }
            _ => {
                let args = Args::new(cmd, 0, &["-d"]).collect_all();
                for p in positionals(&args) {
                    out.input(cmd.path(p));
                }
                Recognition::Partial
            }
        }
    }
}

/// Commands with no file inputs worth recording: `echo`, `printf`, `sleep`,
/// `test`, `[`, `rm`, `rimraf`, `del`, `chmod`, `kill`, `wait`, `pwd`, `exit`,
/// `clear`, `open`.
pub struct Ignore;

impl Recognizer for Ignore {
    fn id(&self) -> &'static str {
        "noop"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "echo", "printf", "sleep", "test", "[", "[[", "rm", "rimraf", "del", "del-cli",
            "trash", "chmod", "chown", "kill", "killall", "pkill", "wait", "pwd", "exit", "clear",
            "open", "xdg-open", "which", "type", "date", "whoami", "hostname", "uname", "false",
            "yes", "wait-on", "shx", "rmdir", "printenv", "tput",
        ]
    }
    fn recognize(&self, _cmd: &Command, _cx: &RecognizeCx<'_>, _out: &mut TaskSpec) -> Recognition {
        Recognition::Ignored
    }
}

/// For `shx <cmd> …`: the command without `shx`, and the real program name.
fn shx(cmd: &Command) -> (Command, String) {
    if cmd.program_name() == Some("shx") {
        let c = cmd.shifted(1);
        let p = c.program_name().unwrap_or("").to_string();
        (c, p)
    } else {
        (cmd.clone(), cmd.program_name().unwrap_or("").to_string())
    }
}
