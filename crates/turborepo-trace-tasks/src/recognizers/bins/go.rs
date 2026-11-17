//! `go tool <name> [args]`: a tool declared in `go.mod` (`tool <path>`
//! directives, Go 1.24+) is a toolchain package (`go:<path>`, versioned by
//! `go.mod`/`go.sum`); a toolchain tool (`cover`, `pprof`, `vet`, ...) is part
//! of the system `go`. What the tool reads is not modelled, so the task also
//! gets the conservative fallback.
//!
//! `go run pkg@version` is already a `go:<pkg>` toolchain package (the `go`
//! recognizer in [`crate::recognizers::run`]).

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::{
        run::go_module_configs,
        util::{add_existing_files, find_up},
    },
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, TaskSpec},
};

/// `go tool <name>`.
pub struct GoTool;

impl Recognizer for GoTool {
    fn id(&self) -> &'static str {
        "go-tool"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["go"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        let mut a = cmd.args();
        cmd.program_name() == Some("go")
            && a.next() == Some("tool")
            && a.next().is_some_and(|n| !n.starts_with('-'))
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("system", "go");
        go_module_configs(&cmd.cwd, cx.repo_root, out);
        let name = cmd.args().nth(1).unwrap_or("");
        let declared = find_up(&cmd.cwd, "go.mod", Some(cx.repo_root))
            .and_then(|m| fsx::read_to_string(m).ok())
            .and_then(|text| declared_tool(&text, name));
        let what = match &declared {
            Some(path) => {
                out.tool("go", path.clone());
                format!("go tool `{path}`")
            }
            None => format!("go tool `{name}`"),
        };
        let rest: Vec<&str> = cmd.args().skip(2).collect();
        add_existing_files(cmd, &rest, out);
        out.uncertain(
            UncertaintyKind::UnknownCommand,
            format!("{what}: inputs not modelled"),
        );
        cx.whole_package(out, &what);
        Recognition::Partial
    }
}

/// The `tool` directive of `go.mod` text whose last path element (ignoring a
/// `/vN` major version suffix) is `name`.
pub fn declared_tool(gomod: &str, name: &str) -> Option<String> {
    let mut in_block = false;
    let mut paths = Vec::new();
    for line in gomod.lines() {
        let t = line.split("//").next().unwrap_or("").trim();
        if in_block {
            if t == ")" {
                in_block = false;
            } else if !t.is_empty() {
                paths.push(t.to_string());
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("tool") {
            let rest = rest.trim();
            if rest == "(" {
                in_block = true;
            } else if !rest.is_empty() && t.starts_with("tool ") {
                paths.push(rest.to_string());
            }
        }
    }
    paths.into_iter().find(|p| {
        let mut segs = p.rsplit('/');
        let last = segs.next().unwrap_or("");
        if last == name {
            return true;
        }
        let is_major = last.len() > 1
            && last.starts_with('v')
            && last[1..].bytes().all(|b| b.is_ascii_digit());
        is_major && segs.next() == Some(name)
    })
}

#[cfg(test)]
mod tests {
    use super::declared_tool;

    #[test]
    fn tool_directives() {
        let m = "module x\n\ngo 1.24\n\ntool golang.org/x/tools/cmd/stringer\n\ntool \
                 (\n\tgithub.com/a/b/v2 // c\n\tgithub.com/c/mockgen\n)\n";
        assert_eq!(
            declared_tool(m, "stringer").as_deref(),
            Some("golang.org/x/tools/cmd/stringer")
        );
        assert_eq!(declared_tool(m, "b").as_deref(), Some("github.com/a/b/v2"));
        assert_eq!(
            declared_tool(m, "mockgen").as_deref(),
            Some("github.com/c/mockgen")
        );
        assert_eq!(declared_tool(m, "cover"), None);
    }
}
