//! Python code generators (`python -m grpc_tools.protoc` is in
//! [`super::protobuf`]).
//!
//! | Program | Inputs | Outputs |
//! |---|---|---|
//! | `datamodel-codegen --input f --output o` | the input schema (+ `$ref`s), `--custom-template-dir`, `--extra-template-data`, `--aliases`, `--custom-file-header-path`; without flags `pyproject.toml [tool.datamodel-codegen]` `input`/`url`/`output`/... | `--output` (stdout otherwise) |
//! | `strawberry export-schema mod[:symbol] [--output f] [--app-dir d]` | the schema module (Python entry) | `--output` |
//! | `strawberry codegen --schema mod -o dir queries…` | the schema module, query files | `-o` |
//! | `strawberry server mod` | the module | |

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;

use super::{
    config::{self, add_spec, as_str, get_path, unknown},
    finish,
};
use crate::{
    recognizers::util::{Args, find_up, opt, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, EntryLang, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&DatamodelCodegen, &Strawberry]
}

/// `datamodel-codegen`.
pub struct DatamodelCodegen;

impl Recognizer for DatamodelCodegen {
    fn id(&self) -> &'static str {
        "datamodel-codegen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["datamodel-codegen", "datamodel_code_generator"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("pypi", "datamodel-code-generator");
        let file_opts = [
            "--custom-template-dir",
            "--extra-template-data",
            "--aliases",
            "--custom-file-header-path",
            "--custom-formatters-kwargs",
        ];
        let mut value_opts = vec![
            "--input",
            "--output",
            "--url",
            "--input-file-type",
            "--output-model-type",
            "--target-python-version",
            "--base-class",
            "--class-name",
            "--http-headers",
            "--encoding",
        ];
        value_opts.extend(file_opts);
        let args = Args::new(cmd, 0, &value_opts).collect_all();
        let mut input = opt(&args, &["--input"]).map(str::to_string);
        let mut url = opt(&args, &["--url"]).map(str::to_string);
        let mut output = opt(&args, &["--output"]).map(|o| cmd.path(o));
        let mut base = cmd.cwd.clone();
        let mut extra: Vec<(String, PathBuf)> = file_opts
            .iter()
            .filter_map(|o| opt(&args, &[o]).map(|v| (o.to_string(), cmd.path(v))))
            .collect();
        if input.is_none() && url.is_none() {
            // pyproject.toml [tool.datamodel-codegen]
            match find_up(&cmd.cwd, "pyproject.toml", Some(cx.repo_root)) {
                Some(pp) => {
                    let dir = config::dir_of(&pp);
                    if let Some(v) = config::load(&pp, out) {
                        let s = |k: &str| {
                            get_path(&v, &["tool", "datamodel-codegen", k])
                                .as_ref()
                                .and_then(as_str)
                        };
                        input = s("input");
                        url = s("url");
                        if output.is_none() {
                            output = s("output").map(|o| fsx::join(&dir, &o));
                        }
                        for o in file_opts {
                            if let Some(x) = s(o.trim_start_matches("--")) {
                                extra.push((o.to_string(), fsx::join(&dir, &x)));
                            }
                        }
                    }
                    base = dir;
                }
                None => out.config(cmd.cwd.join("pyproject.toml")),
            }
        }
        for (o, p) in extra {
            if o == "--custom-template-dir" {
                out.membership(p, "**", "datamodel-codegen templates");
            } else {
                out.input(p);
            }
        }
        match (input, url) {
            (Some(i), _) => {
                add_spec(&base, &i, &base, out);
            }
            (None, Some(u)) => unknown(
                out,
                &base,
                UncertaintyKind::EnvDependent,
                format!("datamodel-codegen remote input `{u}`"),
            ),
            (None, None) => unknown(
                out,
                &base,
                UncertaintyKind::ParseIncomplete,
                "datamodel-codegen without --input (stdin)",
            ),
        }
        if let Some(o) = output {
            out.output(o);
        }
        finish(out, before)
    }
}

/// `strawberry`.
pub struct Strawberry;

impl Recognizer for Strawberry {
    fn id(&self) -> &'static str {
        "strawberry"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["strawberry"]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("pypi", "strawberry-graphql");
        let args = Args::new(
            cmd,
            0,
            &[
                "--output",
                "-o",
                "--output-dir",
                "--app-dir",
                "--schema",
                "-p",
                "--plugins",
                "--cli-plugin",
                "--host",
                "--port",
                "-h",
                "--log-level",
                "--selected-schema",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let sub = pos.first().copied().unwrap_or("");
        let app_dir = opt(&args, &["--app-dir"])
            .map(|d| cmd.path(d))
            .unwrap_or_else(|| cmd.cwd.clone());
        match sub {
            "export-schema" | "server" => {
                match pos.get(1) {
                    Some(m) => module_entry(&app_dir, m, out),
                    None => unknown(
                        out,
                        &cmd.cwd,
                        UncertaintyKind::ParseIncomplete,
                        format!("strawberry {sub} without a schema module"),
                    ),
                }
                if sub == "export-schema"
                    && let Some(o) = opt(&args, &["--output", "-o"])
                {
                    out.output(cmd.path(o));
                }
            }
            "codegen" => {
                match opt(&args, &["--schema"]) {
                    Some(m) => module_entry(&app_dir, m, out),
                    None => unknown(
                        out,
                        &cmd.cwd,
                        UncertaintyKind::ParseIncomplete,
                        "strawberry codegen without --schema",
                    ),
                }
                for q in &pos[1..] {
                    crate::recognizers::util::add_path_input(cmd, q, "graphql queries", out);
                }
                if let Some(o) = opt(&args, &["--output-dir", "-o"]) {
                    out.output(cmd.path(o));
                }
            }
            "" | "--version" | "upgrade" => return Recognition::Ignored,
            other => unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::UnknownCommand,
                format!("strawberry {other}"),
            ),
        }
        finish(out, before)
    }
}

/// `pkg.mod[:symbol]` → `<dir>/pkg/mod.py` or `<dir>/pkg/mod/__init__.py` (a
/// Python entry).
fn module_entry(dir: &Path, m: &str, out: &mut TaskSpec) {
    let module = m.split(':').next().unwrap_or(m);
    let rel = module.replace('.', "/");
    let file = dir.join(format!("{rel}.py"));
    let init = dir.join(&rel).join("__init__.py");
    let p = if fsx::is_file(&file) || !fsx::is_file(&init) {
        file
    } else {
        init
    };
    out.entry(p, EntryLang::Python);
}
