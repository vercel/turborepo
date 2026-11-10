//! `//go:generate` directives as tasks.
//!
//! In a package whose root has a `go.mod`, every `.go`
//! file below it (Go's `./...` rules: no `vendor`, `testdata`, `_*`, `.*`
//! directories, no nested modules or nested workspace packages) is scanned with
//! a cheap byte search for directives
//! ([`crate::recognizers::codegen::golang::directives`]). Each directive
//! becomes a task `gogenerate:<repo-relative file>:<line>` running in the
//! file's directory with `$GOFILE`, `$GOLINE`, `$GOPACKAGE` and `$DOLLAR` set;
//! the file is its defining file (a config input). The scan itself is not
//! recorded as a discovery observation (see [`GoGenerateSource`]). The command
//! goes through the registry like any script, so `stringer`, `mockgen`, `go run
//! ./gen`, `protoc`, ... are recognized by the code generator recognizers.

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::fsx;

use crate::{
    recognizers::codegen::golang::{directive_env, directives, skip_go_dir},
    sources::{DiscoveredTask, SourceCx, TaskSource},
    spec::TaskId,
};

/// Task sources of this module.
pub fn sources() -> Vec<&'static dyn TaskSource> {
    vec![&GoGenerateSource]
}

/// The `//go:generate` task source.
pub struct GoGenerateSource;

impl TaskSource for GoGenerateSource {
    fn id(&self) -> &'static str {
        "gogenerate"
    }

    fn config_file_names(&self) -> &'static [&'static str] {
        &["go.mod"]
    }

    fn discover(&self, cx: &SourceCx<'_>) -> Vec<DiscoveredTask> {
        let root = &cx.package.dir;
        if !fsx::is_file(&root.join("go.mod")) {
            return Vec::new();
        }
        // The scan is not recorded: discovery observations are shared by every task of
        // the package, and a directive task depends only on its own file (a
        // config input) and on what its command reads. Adding or editing a
        // `.go` file re-discovers the package anyway, since the file belongs to
        // it.
        let scanned: Vec<(PathBuf, Vec<u8>)> = turborepo_trace_core::record::pause(|| {
            let mut files = Vec::new();
            walk(cx, root, 0, &mut files);
            files.sort();
            files
                .into_iter()
                .filter_map(|f| {
                    let src = fsx::read(&f).ok()?;
                    (memchr::memmem::find(&src, b"//go:generate").is_some()).then_some((f, src))
                })
                .collect()
        });
        let mut out = Vec::new();
        for (f, src) in scanned {
            let ds = directives(&src);
            if ds.is_empty() {
                continue;
            }
            let rel = cx.workspace.rel(&f);
            let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
            let fname = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for d in ds {
                let mut t = DiscoveredTask::new(
                    TaskId::other("gogenerate", format!("{rel}:{}", d.line)),
                    format!("{fname}:{}", d.line),
                    dir.clone(),
                );
                t.lines.push(d.command.clone());
                t.env = directive_env(&f, d.line, &src);
                t.files.push(f.clone());
                out.push(t);
            }
        }
        out
    }
}

/// `.go` files below `dir` that belong to this package and module.
fn walk(cx: &SourceCx<'_>, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Some(entries) = fsx::read_dir(dir) else {
        return;
    };
    for (p, is_dir) in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_dir {
            if depth < 32 && !skip_go_dir(name) && !fsx::is_file(&p.join("go.mod")) && cx.owns(&p) {
                walk(cx, &p, depth + 1, out);
            }
        } else if name.ends_with(".go") && !name.starts_with('.') && !name.starts_with('_') {
            out.push(p);
        }
    }
}
