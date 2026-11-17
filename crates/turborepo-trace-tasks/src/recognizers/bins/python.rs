//! Python console scripts of first-party projects: `[project.scripts]`,
//! `[project.gui-scripts]` and `[tool.poetry.scripts]` entries (`name =
//! "pkg.mod:func"`) of the `pyproject.toml` files from the cwd up to the repo
//! root, plus the members of a uv workspace declared there
//! (`[tool.uv.workspace] members`, one glob level).
//!
//! The module file is found with the module-path heuristic:
//! `<project>/src/<a>/<b>.py`, `<project>/<a>/<b>.py`, or the package
//! `__init__.py`.

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::{
    fsx,
    iniconf::{IniValue, toml_table},
};

use crate::{registry::RecognizeCx, spec::Command};

/// A resolved console script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleScript {
    /// The defining `pyproject.toml`.
    pub pyproject: PathBuf,
    /// `pkg.mod:func` as written.
    pub target: String,
    /// The module file, if found.
    pub module_file: Option<PathBuf>,
    /// The most likely module path (a probe when not found).
    pub probe: PathBuf,
}

/// The console script `name`, if a first-party pyproject declares it.
pub fn find(name: &str, cmd: &Command, cx: &RecognizeCx<'_>) -> Option<ConsoleScript> {
    let start: &Path = if cmd.cwd.starts_with(cx.repo_root) {
        &cmd.cwd
    } else {
        cx.package_root
    };
    let mut projects: Vec<PathBuf> = Vec::new();
    for d in start
        .ancestors()
        .take_while(|d| d.starts_with(cx.repo_root))
    {
        let p = d.join("pyproject.toml");
        if fsx::is_file(&p) {
            projects.push(p);
        }
    }
    let mut members = Vec::new();
    for p in &projects {
        if let Ok(text) = fsx::read_to_string(p)
            && let Some(t) = toml_table(&text, "tool.uv.workspace")
        {
            let dir = p.parent().unwrap_or(cx.repo_root);
            for (k, v) in t {
                if k == "members"
                    && let IniValue::List(globs) = v
                {
                    for g in globs {
                        members.extend(expand_one_level(dir, &g));
                    }
                }
            }
        }
    }
    for m in members {
        if !projects.contains(&m) {
            projects.push(m);
        }
    }
    for p in projects {
        let Ok(text) = fsx::read_to_string(&p) else {
            continue;
        };
        for table in [
            "project.scripts",
            "project.gui-scripts",
            "tool.poetry.scripts",
        ] {
            let Some(entries) = toml_table(&text, table) else {
                continue;
            };
            for (k, v) in entries {
                if k != name {
                    continue;
                }
                let IniValue::Str(target) = v else { continue };
                let dir = p.parent().unwrap_or(cx.repo_root).to_path_buf();
                let module = target.split(':').next().unwrap_or("").trim();
                if module.is_empty() || module.starts_with('{') {
                    continue;
                }
                let (module_file, probe) = module_file(&dir, module);
                return Some(ConsoleScript {
                    pyproject: p.clone(),
                    target: target.clone(),
                    module_file,
                    probe,
                });
            }
        }
    }
    None
}

/// `dir/<glob>` with `*` only in the last segment (`packages/*`), or a plain
/// path; the `pyproject.toml` of each match.
fn expand_one_level(dir: &Path, g: &str) -> Vec<PathBuf> {
    let g = g.trim_end_matches('/');
    match g.rsplit_once('/') {
        Some((parent, "*")) if !parent.contains('*') => fsx::read_dir(&fsx::join(dir, parent))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, d)| *d)
            .map(|(p, _)| p.join("pyproject.toml"))
            .filter(|p| fsx::is_file(p))
            .collect(),
        _ if g == "*" => fsx::read_dir(dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, d)| *d)
            .map(|(p, _)| p.join("pyproject.toml"))
            .filter(|p| fsx::is_file(p))
            .collect(),
        _ if !g.contains('*') => {
            let p = fsx::join(dir, g).join("pyproject.toml");
            if fsx::is_file(&p) {
                vec![p]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// The module file of `module` (dotted) under a project directory, and the
/// probe to record when it is missing.
pub fn module_file(dir: &Path, module: &str) -> (Option<PathBuf>, PathBuf) {
    let rel = module.replace('.', "/");
    let mut cands = Vec::new();
    for base in [dir.join("src"), dir.to_path_buf()] {
        cands.push(base.join(format!("{rel}.py")));
        cands.push(base.join(&rel).join("__init__.py"));
    }
    let found = cands.iter().find(|c| fsx::is_file(c)).cloned();
    (found, dir.join(format!("{rel}.py")))
}
