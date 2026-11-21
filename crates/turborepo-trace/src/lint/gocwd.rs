//! The rule `go-working-directory`: a Go file that changes the working
//! directory of the process.
//!
//! `go test` runs a package's tests in the package directory, and turbo trace
//! takes their relative paths from there: a literal read of a test file is a
//! reference to the file next to the package, and a string that climbs with
//! parent steps (`"../../shared/a.json"`) names its rest at the directories it
//! climbs to (`turborepo_trace_lang_go::names`, "Strings that climb"). A call
//! of `Chdir` (`os.Chdir`, `t.Chdir`; not into a temporary directory) moves
//! every relative path read after it. Inside the package that is followed: the
//! call is a read with an unknown path that every test of the directory
//! reaches, so `--strict` opens it, and by default the names of the
//! directory's Go files name their shape anywhere. What is left is assumed: a
//! string of another package (a helper) read relative to where the test moved
//! is held to its package, and a package that changes the working directory
//! in its `init` moves the paths of every test that imports it, which nothing
//! follows. This rule lists every such call, so that both can be checked.
//!
//! The key of a finding is the file's directory.

use anyhow::Result;

use super::{Ctx, Finding};

#[cfg_attr(not(feature = "go"), allow(dead_code))]
const RULE: &str = "go-working-directory";

/// The findings of `go-working-directory`.
pub fn changes(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    #[cfg(not(feature = "go"))]
    {
        let _ = ctx;
        Ok(Vec::new())
    }
    #[cfg(feature = "go")]
    {
        use turborepo_trace_graph::{GraphView, NodeId, UncertaintyKind};
        use turborepo_trace_lang_go::extract::CHDIR_DETAIL as DETAIL;
        let g = ctx.graph;
        let mut out = Vec::new();
        for i in 0..g.node_count() {
            let n = NodeId(i);
            if g.lang(n) != Some("go") || g.is_external(n) {
                continue;
            }
            let path = g.path(n);
            let dir = path.rsplit_once('/').map_or("", |x| x.0);
            let test = path.ends_with("_test.go");
            for u in g.uncertainties(n) {
                if u.kind != UncertaintyKind::CwdDependent || u.detail != DETAIL {
                    continue;
                }
                let at = if dir.is_empty() {
                    "the repository root".to_owned()
                } else {
                    format!("{dir}/")
                };
                out.push(Finding {
                    rule: RULE,
                    file: path.to_owned(),
                    line: u.line,
                    key: dir.to_owned(),
                    message: if test {
                        format!(
                            "the test changes the working directory: the relative paths of the Go \
                             files of {at} are taken from where it moves, and their strings name \
                             paths anywhere; a string of another package read after the change is \
                             still held to that package"
                        )
                    } else {
                        format!(
                            "a Go file of {at} that is no test changes the working directory: the \
                             tests of this package follow it, the tests of the packages that \
                             import it are taken to run where go test starts them"
                        )
                    },
                    cost: None,
                    fix: "Build the path on the package directory (`filepath.Join(dir, …)` with \
                          `dir` from `os.Getwd` before the change), or select with `--strict`."
                        .to_owned(),
                });
            }
        }
        Ok(out)
    }
}
