//! OpenAPI client and type generators. Every local spec is an input together
//! with the files its `$ref`s reach (transitively); remote specs (URLs) are
//! uncertain. `openapi-typescript` and `orval` are declarative specs
//! (`crates/turborepo-trace-tasks/specs/`).
//!
//! | Program | Inputs | Outputs |
//! |---|---|---|
//! | `openapi-generator(-cli) generate -i spec -o out -g gen [-c cfg] [-t tpl]` | spec, config (`inputSpec`, `outputDir`, `templateDir`), templates, `<out>/.openapi-generator-ignore`; `openapitools.json` `generator-cli.generators.*` without `-i` | `-o` (default: the cwd) |
//! | `openapi-ts [-f cfg] [-i in] [-o out]` (`@hey-api/openapi-ts`) | `openapi-ts.config.*` `input` (string, `{ path }`, list) | `output` directory |
//! | `swagger-typescript-api [generate] -p spec -o dir [-n name] [--modular] [-t tpl]` | spec, templates | `<dir>/<name>` (default `Api.ts`), or `<dir>` with `--modular` |
//! | `openapi --input spec --output dir` (openapi-typescript-codegen) | spec | `<dir>` |

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;
use turborepo_trace_lang_js::extract::config::ConfigVal;

use super::{
    config::{self, add_spec, as_str, entries, get, unknown},
    finish,
};
use crate::{
    recognizers::util::{Args, flag, opt, positionals},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![
        &OpenapiGenerator,
        &HeyApi,
        &SwaggerTypescriptApi,
        &OpenapiCodegen,
    ]
}

/// `openapi-generator-cli` / `openapi-generator`.
pub struct OpenapiGenerator;

impl Recognizer for OpenapiGenerator {
    fn id(&self) -> &'static str {
        "openapi-generator"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["openapi-generator-cli", "openapi-generator"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        if cmd.program_name() == Some("openapi-generator-cli") {
            out.tool("npm", "@openapitools/openapi-generator-cli");
        } else {
            out.tool("system", "openapi-generator");
        }
        let args = Args::new(
            cmd,
            0,
            &[
                "-i",
                "--input-spec",
                "-o",
                "--output",
                "-g",
                "--generator-name",
                "-c",
                "--config",
                "-t",
                "--template-dir",
                "--ignore-file-override",
                "--generator-key",
                "-p",
                "--additional-properties",
                "--package-name",
                "--api-package",
                "--model-package",
                "--type-mappings",
                "--import-mappings",
                "--global-property",
                "--custom-generator",
                "-e",
                "--engine",
                "--auth",
                "-a",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let sub = pos.first().copied().unwrap_or("");
        // openapitools.json (the npm wrapper's version pin and generator presets).
        let tools_json = cmd.cwd.join("openapitools.json");
        if cmd.program_name() == Some("openapi-generator-cli") {
            out.config(tools_json.clone());
        }
        match sub {
            "generate" => {}
            "validate" => {
                if let Some(i) = opt(&args, &["-i", "--input-spec"]) {
                    add_spec(&cmd.cwd, i, &cmd.cwd, out);
                }
                return finish(out, before);
            }
            "batch" => {
                for p in &pos[1..] {
                    crate::recognizers::util::add_path_input(cmd, p, "openapi batch config", out);
                }
                unknown(
                    out,
                    &cmd.cwd,
                    UncertaintyKind::ParseIncomplete,
                    "openapi-generator batch outputs",
                );
                return finish(out, before);
            }
            "" | "help" | "list" | "version" | "version-manager" | "config-help" | "meta"
            | "author" | "completion" => return Recognition::Ignored,
            other => {
                unknown(
                    out,
                    cx.package_root,
                    UncertaintyKind::UnknownCommand,
                    format!("openapi-generator {other}"),
                );
                return finish(out, before);
            }
        }
        // Config file values are defaults for the flags.
        let cfg = opt(&args, &["-c", "--config"]).map(|c| cmd.path(c));
        let cv = cfg.as_ref().and_then(|c| config::load(c, out));
        let cfg_str = |k: &str| {
            cv.as_ref()
                .and_then(|v| get(v, k))
                .as_ref()
                .and_then(as_str)
        };
        let input = opt(&args, &["-i", "--input-spec"])
            .map(str::to_string)
            .or_else(|| cfg_str("inputSpec"));
        let output = opt(&args, &["-o", "--output"])
            .map(str::to_string)
            .or_else(|| cfg_str("outputDir"));
        let tpl = opt(&args, &["-t", "--template-dir"])
            .map(str::to_string)
            .or_else(|| cfg_str("templateDir"));
        if let Some(t) = tpl {
            out.membership(cmd.path(&t), "**", "openapi-generator templates");
        }
        if let Some(f) = opt(&args, &["--ignore-file-override"]) {
            out.input(cmd.path(f));
        }
        match input {
            Some(i) => {
                add_spec(&cmd.cwd, &i, &cmd.cwd, out);
                let o = output
                    .map(|o| cmd.path(&o))
                    .unwrap_or_else(|| cmd.cwd.clone());
                out.input(o.join(".openapi-generator-ignore"));
                out.output(o);
            }
            None => {
                // openapitools.json generators.
                let key = opt(&args, &["--generator-key"]);
                let gens = config::load(&tools_json, out)
                    .and_then(|v| get(&v, "generator-cli"))
                    .and_then(|g| get(&g, "generators"))
                    .map(|g| entries(&g));
                match gens {
                    Some(Ok(gens)) if !gens.is_empty() => {
                        for (name, g) in gens {
                            if key.is_some_and(|k| !k.split(',').any(|x| x == name)) {
                                continue;
                            }
                            let s = |k: &str| {
                                get(&g, k)
                                    .as_ref()
                                    .and_then(as_str)
                                    .map(|x| x.replace("#{cwd}/", "").replace("#{cwd}", "."))
                            };
                            match s("inputSpec") {
                                Some(i) => {
                                    add_spec(&cmd.cwd, &i, &cmd.cwd, out);
                                }
                                None => {
                                    if let Some(gl) = s("glob") {
                                        crate::recognizers::util::add_glob(
                                            &cmd.cwd,
                                            &gl,
                                            "openapi specs",
                                            out,
                                        );
                                    } else {
                                        unknown(
                                            out,
                                            &cmd.cwd,
                                            UncertaintyKind::ParseIncomplete,
                                            format!(
                                                "openapitools generator `{name}` without inputSpec"
                                            ),
                                        );
                                    }
                                }
                            }
                            match s("output") {
                                Some(o) => {
                                    let o = fsx::join(&cmd.cwd, &o);
                                    out.input(o.join(".openapi-generator-ignore"));
                                    out.output(o);
                                }
                                None => out.output(cmd.cwd.clone()),
                            }
                            if let Some(t) = s("templateDir") {
                                out.membership(cmd.path(&t), "**", "openapi-generator templates");
                            }
                        }
                    }
                    _ => unknown(
                        out,
                        &cmd.cwd,
                        UncertaintyKind::ParseIncomplete,
                        "openapi-generator generate without an input spec",
                    ),
                }
            }
        }
        finish(out, before)
    }
}

/// `openapi-ts` (`@hey-api/openapi-ts`).
pub struct HeyApi;

impl Recognizer for HeyApi {
    fn id(&self) -> &'static str {
        "hey-api"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["openapi-ts"]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("npm", "@hey-api/openapi-ts");
        let args = Args::new(
            cmd,
            0,
            &[
                "-i",
                "--input",
                "-o",
                "--output",
                "-f",
                "--file",
                "-c",
                "--client",
                "-p",
                "--plugins",
                "-l",
                "--logs",
            ],
        )
        .collect_all();
        if let (Some(i), Some(o)) = (
            opt(&args, &["-i", "--input"]),
            opt(&args, &["-o", "--output"]),
        ) {
            add_spec(&cmd.cwd, i, &cmd.cwd, out);
            out.output(cmd.path(o));
            return finish(out, before);
        }
        let cfg = match opt(&args, &["-f", "--file"]) {
            Some(f) => Some(cmd.path(f)),
            None => fsx::first_existing(
                &cmd.cwd,
                &[
                    "openapi-ts.config.ts",
                    "openapi-ts.config.mts",
                    "openapi-ts.config.cts",
                    "openapi-ts.config.js",
                    "openapi-ts.config.mjs",
                    "openapi-ts.config.cjs",
                ],
            ),
        };
        let Some(cfg) = cfg else {
            out.config(cmd.cwd.join("openapi-ts.config.ts"));
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "openapi-ts without a config or -i/-o",
            );
            return finish(out, before);
        };
        let Some(v) = config::load(&cfg, out) else {
            unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "openapi-ts config could not be evaluated",
            );
            return finish(out, before);
        };
        let items: Vec<ConfigVal> = match v {
            ConfigVal::Arr(a) => a,
            other => vec![other],
        };
        let cli_in = opt(&args, &["-i", "--input"]);
        let cli_out = opt(&args, &["-o", "--output"]);
        for it in items {
            let path_of = |key: &str| -> (Vec<String>, Vec<String>) {
                match get(&it, key) {
                    None => (Vec::new(), vec![format!("no `{key}`")]),
                    Some(x) => {
                        let xs: Vec<ConfigVal> = match &x {
                            ConfigVal::Arr(a) => a.clone(),
                            o => vec![o.clone()],
                        };
                        let mut ok = Vec::new();
                        let mut bad = Vec::new();
                        for e in xs {
                            match as_str(&e).or_else(|| get(&e, "path").as_ref().and_then(as_str)) {
                                Some(s) => ok.push(s),
                                None => bad.push(format!("`{key}`: {}", config::describe(&e))),
                            }
                        }
                        (ok, bad)
                    }
                }
            };
            let (ins, bad_in) = match cli_in {
                Some(i) => (vec![i.to_string()], Vec::new()),
                None => path_of("input"),
            };
            let (outs, bad_out) = match cli_out {
                Some(o) => (vec![o.to_string()], Vec::new()),
                None => path_of("output"),
            };
            for i in ins {
                add_spec(&cmd.cwd, &i, &cmd.cwd, out);
            }
            for o in outs {
                out.output(fsx::join(&cmd.cwd, &o));
            }
            for b in bad_in.into_iter().chain(bad_out) {
                unknown(
                    out,
                    &config::dir_of(&cfg),
                    UncertaintyKind::EnvDependent,
                    format!("openapi-ts {b}"),
                );
            }
        }
        finish(out, before)
    }
}

/// `swagger-typescript-api`.
pub struct SwaggerTypescriptApi;

impl Recognizer for SwaggerTypescriptApi {
    fn id(&self) -> &'static str {
        "swagger-typescript-api"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["swagger-typescript-api", "sta"]
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("npm", "swagger-typescript-api");
        let args = Args::new(
            cmd,
            0,
            &[
                "-p",
                "--path",
                "-o",
                "--output",
                "-n",
                "--name",
                "-t",
                "--templates",
                "--custom-config",
                "--api-class-name",
                "--default-response",
                "--type-prefix",
                "--type-suffix",
                "--module-name-index",
                "--http-client",
            ],
        )
        .collect_all();
        match opt(&args, &["-p", "--path"]) {
            Some(p) => {
                add_spec(&cmd.cwd, p, &cmd.cwd, out);
            }
            None => unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "swagger-typescript-api without --path",
            ),
        }
        if let Some(t) = opt(&args, &["-t", "--templates"]) {
            out.membership(cmd.path(t), "**", "swagger-typescript-api templates");
        }
        if let Some(c) = opt(&args, &["--custom-config"]) {
            crate::recognizers::util::add_code_config(&cmd.path(c), out);
        }
        let dir = cmd.path(opt(&args, &["-o", "--output"]).unwrap_or("./"));
        if flag(&args, &["--modular"]) || flag(&args, &["--extract-request-params"]) {
            out.output(dir);
        } else {
            let name = opt(&args, &["-n", "--name"]).unwrap_or("Api.ts");
            out.output(dir.join(name));
        }
        finish(out, before)
    }
}

/// `openapi --input spec --output dir` (openapi-typescript-codegen).
pub struct OpenapiCodegen;

impl Recognizer for OpenapiCodegen {
    fn id(&self) -> &'static str {
        "openapi-typescript-codegen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["openapi", "openapi-typescript-codegen"]
    }
    fn matches(&self, cmd: &Command) -> bool {
        match cmd.program_name() {
            Some("openapi-typescript-codegen") => true,
            Some("openapi") => cmd
                .args()
                .any(|a| a == "-i" || a == "--input" || a.starts_with("--input=")),
            _ => false,
        }
    }
    fn recognize(&self, cmd: &Command, _cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("npm", "openapi-typescript-codegen");
        let args = Args::new(
            cmd,
            0,
            &[
                "-i",
                "--input",
                "-o",
                "--output",
                "-c",
                "--client",
                "--name",
                "--request",
                "--postfixServices",
                "--postfixModels",
            ],
        )
        .collect_all();
        if let Some(i) = opt(&args, &["-i", "--input"]) {
            add_spec(&cmd.cwd, i, &cmd.cwd, out);
        }
        if let Some(r) = opt(&args, &["--request"]) {
            out.input(cmd.path(r));
        }
        match opt(&args, &["-o", "--output"]) {
            Some(o) => out.output(cmd.path(o)),
            None => unknown(
                out,
                &cmd.cwd,
                UncertaintyKind::ParseIncomplete,
                "openapi-typescript-codegen without --output",
            ),
        }
        finish(out, before)
    }
}
