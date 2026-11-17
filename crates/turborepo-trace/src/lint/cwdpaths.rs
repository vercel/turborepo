//! The rule `cwd-path-explained`: a path relative to the working directory
//! whose reading from the repository root was not taken.
//!
//! A test runner can be started in the repository root, so a path relative to
//! the working directory that climbs above the root and comes back down
//! (`../ui/data/h.json`) has a reading there: `data/h.json`, in a checkout
//! whose directory is called `ui`. The per-test resolution takes that reading,
//! except where another directory explains the path: from a working directory
//! of the test in a package, or from the package that holds the reference, the
//! same path stays inside the repository and goes through an existing directory
//! (`packages/ui`). The path is then taken to be written for that directory and
//! is not also read from the root (`turborepo_trace_store::Graph::cwd_patch`).
//! That is an assumption: a repository is not checked out below directories
//! whose names repeat a path inside it. `--strict` and
//! `TURBO_TRACE_DEBUG_CWD_EXPLAINED=0` take every reading, `--explain`
//! says where the assumption decided for a test, and this rule lists every
//! reference it decided for. Off by default: on two large repositories it
//! reports 15 and 19 references (4 and 8 files), none of them in a checkout the
//! assumption fails for.
//!
//! The key of a finding is the path as written.

use anyhow::Result;

use super::{Ctx, Finding};

const RULE: &str = "cwd-path-explained";

/// The findings of `cwd-path-explained`.
pub fn explained(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    Ok(ctx
        .graph
        .cwd_dropped()
        .into_iter()
        .map(|d| Finding {
            rule: RULE,
            file: d.site,
            line: d.line,
            key: d.spec.clone(),
            message: format!(
                "`{}` is read relative to the working directory; from the repository root it \
                 names `{}` only if {}. It is taken to be written for {}/, from where it goes \
                 through {}/, and is not read from the root",
                d.spec, d.names, d.when, d.dir, d.at
            ),
            cost: None,
            fix: "Anchor the path to the file (`path.join(__dirname, …)`), or select with \
                  `--strict`, which reads it from the root as well."
                .to_owned(),
        })
        .collect())
}
