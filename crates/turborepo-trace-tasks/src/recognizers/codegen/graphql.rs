//! GraphQL Code Generator: `graphql-codegen`, `gql-gen`,
//! `graphql-code-generator`, `graphql-codegen-esm`.
//!
//! Config: `-c/--config <file>`, else the first of
//! `codegen.{json,yml,yaml,js,ts,mjs,mts,cjs,cts}`, `.codegenrc{,.json,.yml,.
//! yaml,.js,.ts}`, `codegen.config.js` in the cwd, else `package.json#codegen`,
//! else graphql-config (`.graphqlrc*`, `graphql.config.*`: its
//! `schema`/`documents` and `extensions.codegen`; `-p/--project` selects a
//! project, all projects otherwise).
//!
//! Fields read (relative to the cwd, as the CLI resolves them):
//! - `schema` (root and per output): files (`.graphql`/`.json` with their
//!   `#import`s; code files are JS entries), globs (memberships), URLs
//!   (uncertain: network), the object form's keys;
//! - `documents` (root and per output): globs with `!` negations;
//! - `generates` keys: output files or directories (`client` preset dirs end
//!   with `/`); `near-operation-file` writes `<presetConfig.extension>`
//!   (default `.generated.ts`) files next to the documents below the key's
//!   directory, optionally in `presetConfig.folder`;
//! - `require` (root): local modules are entries, packages toolchain.

use std::path::Path;

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::fsx;
use turborepo_trace_lang_js::extract::config::ConfigVal;

use super::{
    config::{
        self, PathRole, add_location, add_spec, as_str, entries, get, output_pattern, str_list,
        unknown,
    },
    finish,
};
use crate::{
    recognizers::util::{Args, opt, opts},
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, EntryLang, TaskSpec},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![&GraphqlCodegen]
}

/// `graphql-codegen …`.
pub struct GraphqlCodegen;

const CODEGEN_NAMES: &[&str] = &[
    "codegen.json",
    "codegen.yml",
    "codegen.yaml",
    "codegen.js",
    "codegen.ts",
    "codegen.mjs",
    "codegen.mts",
    "codegen.cjs",
    "codegen.cts",
    ".codegenrc",
    ".codegenrc.json",
    ".codegenrc.yml",
    ".codegenrc.yaml",
    ".codegenrc.js",
    ".codegenrc.ts",
    "codegen.config.js",
];

const GRAPHQL_CONFIG_NAMES: &[&str] = &[
    ".graphqlrc",
    ".graphqlrc.json",
    ".graphqlrc.yml",
    ".graphqlrc.yaml",
    ".graphqlrc.ts",
    ".graphqlrc.js",
    "graphql.config.json",
    "graphql.config.yml",
    "graphql.config.yaml",
    "graphql.config.ts",
    "graphql.config.js",
    "graphql.config.cjs",
];

impl Recognizer for GraphqlCodegen {
    fn id(&self) -> &'static str {
        "graphql-codegen"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "graphql-codegen",
            "gql-gen",
            "graphql-code-generator",
            "graphql-codegen-esm",
        ]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let before = out.uncertainties.len();
        out.tool("npm", "@graphql-codegen/cli");
        let args = Args::new(
            cmd,
            0,
            &["-c", "--config", "-p", "--project", "-r", "--require"],
        )
        .collect_all();
        if matches!(
            crate::recognizers::util::positionals(&args).first(),
            Some(&"init")
        ) {
            return Recognition::Ignored;
        }
        for r in opts(&args, &["-r", "--require"]) {
            require(cmd, r, out);
        }
        let project = opt(&args, &["-p", "--project"]);
        let (cfg, value) = match opt(&args, &["-c", "--config"]) {
            Some(c) => {
                let p = cmd.path(c);
                let v = config::load(&p, out);
                (Some(p), v)
            }
            None => find_config(cmd, out),
        };
        let cfg_dir = cfg
            .as_ref()
            .map(|c| config::dir_of(c))
            .unwrap_or_else(|| cmd.cwd.clone());
        let Some(value) = value else {
            unknown(
                out,
                &cfg_dir,
                UncertaintyKind::ParseIncomplete,
                match &cfg {
                    Some(c) => format!(
                        "graphql-codegen config `{}` not readable",
                        config::file_name(c)
                    ),
                    None => "no graphql-codegen config found".to_string(),
                },
            );
            return finish(out, before);
        };
        let is_graphql_config = cfg.as_ref().is_some_and(|c| {
            let n = config::file_name(c);
            n.starts_with(".graphqlrc") || n.starts_with("graphql.config")
        });
        if is_graphql_config {
            graphql_config(cmd, &value, project, &cfg_dir, out);
        } else {
            codegen_config(cmd, &value, None, None, &cfg_dir, out);
        }
        let _ = cx;
        finish(out, before)
    }
}

/// The codegen config in the cwd, `package.json#codegen`, or graphql-config.
fn find_config(
    cmd: &Command,
    out: &mut TaskSpec,
) -> (Option<std::path::PathBuf>, Option<ConfigVal>) {
    if let Some(p) = fsx::first_existing(&cmd.cwd, CODEGEN_NAMES) {
        let v = config::load(&p, out);
        return (Some(p), v);
    }
    for n in CODEGEN_NAMES.iter().take(5) {
        out.config(cmd.cwd.join(n));
    }
    let pj = cmd.cwd.join("package.json");
    out.config(pj.clone());
    if let Some(m) = crate::sources::read_json(&pj)
        && let Some(c) = m.get("codegen")
    {
        return (Some(pj), Some(config::from_json(c)));
    }
    if let Some(p) = fsx::first_existing(&cmd.cwd, GRAPHQL_CONFIG_NAMES) {
        let v = config::load(&p, out);
        return (Some(p), v);
    }
    out.config(cmd.cwd.join(".graphqlrc.yml"));
    (None, None)
}

/// graphql-config: `schema`/`documents` per project plus `extensions.codegen`.
fn graphql_config(
    cmd: &Command,
    v: &ConfigVal,
    project: Option<&str>,
    cfg_dir: &Path,
    out: &mut TaskSpec,
) {
    let projects: Vec<ConfigVal> = match get(v, "projects") {
        Some(ps) => match entries(&ps) {
            Ok(es) => es
                .into_iter()
                .filter(|(k, _)| project.is_none_or(|p| p == k))
                .map(|(_, v)| v)
                .collect(),
            Err(r) => {
                unknown(
                    out,
                    cfg_dir,
                    UncertaintyKind::EnvDependent,
                    format!("graphql-config projects: {r}"),
                );
                Vec::new()
            }
        },
        None => vec![v.clone()],
    };
    for p in projects {
        let codegen = p
            .field("extensions")
            .and_then(|e| get(&e, "codegen"))
            .unwrap_or(ConfigVal::Null);
        if codegen.is_null() {
            unknown(
                out,
                cfg_dir,
                UncertaintyKind::ParseIncomplete,
                "graphql-config without extensions.codegen",
            );
            continue;
        }
        codegen_config(
            cmd,
            &codegen,
            get(&p, "schema").as_ref(),
            get(&p, "documents").as_ref(),
            cfg_dir,
            out,
        );
    }
}

/// A codegen config object (`schema`, `documents`, `generates`, `require`).
fn codegen_config(
    cmd: &Command,
    v: &ConfigVal,
    outer_schema: Option<&ConfigVal>,
    outer_docs: Option<&ConfigVal>,
    cfg_dir: &Path,
    out: &mut TaskSpec,
) {
    let base = &cmd.cwd;
    if let Some(s) = outer_schema {
        schema(base, s, cfg_dir, out);
    }
    if let Some(d) = outer_docs {
        documents(base, d, cfg_dir, out);
    }
    if let Some(s) = get(v, "schema") {
        schema(base, &s, cfg_dir, out);
    }
    let docs = get(v, "documents");
    if let Some(d) = &docs {
        documents(base, d, cfg_dir, out);
    }
    if let Some(r) = get(v, "require") {
        for m in str_list(&r).0 {
            require(cmd, &m, out);
        }
    }
    let Some(generates) = get(v, "generates") else {
        unknown(
            out,
            cfg_dir,
            UncertaintyKind::ParseIncomplete,
            "graphql-codegen config without `generates`",
        );
        return;
    };
    let gens = match entries(&generates) {
        Ok(g) => g,
        Err(r) => {
            unknown(
                out,
                cfg_dir,
                UncertaintyKind::EnvDependent,
                format!("graphql-codegen `generates`: {r}"),
            );
            return;
        }
    };
    for (path, opts) in gens {
        if path.contains("${") {
            unknown(
                out,
                cfg_dir,
                UncertaintyKind::EnvDependent,
                format!("graphql-codegen output `{path}` uses a variable"),
            );
            continue;
        }
        let target = fsx::join(base, &path);
        if let Some(s) = get(&opts, "schema") {
            schema(base, &s, cfg_dir, out);
        }
        if let Some(d) = get(&opts, "documents") {
            documents(base, &d, cfg_dir, out);
        }
        for p in get(&opts, "plugins")
            .or_else(|| matches!(opts, ConfigVal::Arr(_)).then(|| opts.clone()))
            .map(|p| plugin_names(&p))
            .unwrap_or_default()
        {
            plugin_tool(cmd, &p, out);
        }
        let preset = get(&opts, "preset").as_ref().and_then(as_str);
        if let Some(p) = &preset {
            plugin_tool(cmd, &format!("preset-{p}"), out);
        }
        match preset.as_deref() {
            Some("near-operation-file" | "near-operation-file-preset") => {
                let pc = get(&opts, "presetConfig").unwrap_or(ConfigVal::Null);
                let ext = get(&pc, "extension")
                    .as_ref()
                    .and_then(as_str)
                    .unwrap_or_else(|| ".generated.ts".into());
                let pattern = match get(&pc, "folder").as_ref().and_then(as_str) {
                    Some(f) => format!("**/{f}/*{ext}"),
                    None => format!("**/*{ext}"),
                };
                output_pattern(out, target, &pattern, &[]);
                if let Some(b) = get(&pc, "baseTypesPath").as_ref().and_then(as_str)
                    && !b.starts_with('~')
                {
                    // Relative to the key directory; an input of the generated files.
                    out.input(fsx::join(&fsx::join(base, &path), &b));
                }
            }
            Some("graphql-modules" | "graphql-modules-preset") => {
                let pc = get(&opts, "presetConfig").unwrap_or(ConfigVal::Null);
                match get(&pc, "filename").as_ref().and_then(as_str) {
                    Some(f) => output_pattern(out, target, &format!("**/{f}"), &[]),
                    None => out.output(target),
                }
                if let Some(b) = get(&pc, "baseTypesPath").as_ref().and_then(as_str) {
                    out.output(fsx::join(base, &b));
                }
            }
            _ => out.output(target),
        }
    }
}

/// `schema`: string, list, or object keyed by location.
fn schema(base: &Path, v: &ConfigVal, cfg_dir: &Path, out: &mut TaskSpec) {
    for loc in locations(v, cfg_dir, "schema", out) {
        if loc.starts_with('!') || crate::glob::is_glob(&loc) || config::is_url(&loc) {
            add_location(
                base,
                &loc,
                PathRole::Code,
                "**",
                "graphql schema",
                cfg_dir,
                out,
            );
            continue;
        }
        let p = fsx::join(base, &loc);
        if EntryLang::of(&p) == EntryLang::Js {
            out.entry(p.clone(), EntryLang::Js);
            out.input(p);
        } else {
            add_spec(base, &loc, cfg_dir, out);
        }
    }
}

/// `documents`: globs with negations.
fn documents(base: &Path, v: &ConfigVal, cfg_dir: &Path, out: &mut TaskSpec) {
    let locs = locations(v, cfg_dir, "documents", out);
    // Positive globs first so negations apply to them.
    let (neg, pos): (Vec<_>, Vec<_>) = locs.into_iter().partition(|l| l.starts_with('!'));
    for l in pos.iter().chain(neg.iter()) {
        add_location(
            base,
            l,
            PathRole::Data,
            "**/*.{graphql,gql,ts,tsx,js,jsx}",
            "graphql documents",
            cfg_dir,
            out,
        );
    }
}

/// The location strings of a `schema`/`documents` value (strings, lists, object
/// keys).
fn locations(v: &ConfigVal, cfg_dir: &Path, what: &str, out: &mut TaskSpec) -> Vec<String> {
    let mut res = Vec::new();
    let items: Vec<ConfigVal> = match v {
        ConfigVal::Arr(a) => a.clone(),
        other => vec![other.clone()],
    };
    for it in items {
        match &it {
            ConfigVal::Obj(_) => match entries(&it) {
                Ok(es) => res.extend(es.into_iter().map(|(k, _)| k)),
                Err(r) => unknown(
                    out,
                    cfg_dir,
                    UncertaintyKind::EnvDependent,
                    format!("graphql-codegen `{what}`: {r}"),
                ),
            },
            _ => {
                let (ss, unk) = str_list(&it);
                res.extend(ss);
                for u in unk {
                    unknown(
                        out,
                        cfg_dir,
                        UncertaintyKind::EnvDependent,
                        format!("graphql-codegen `{what}`: {u}"),
                    );
                }
            }
        }
    }
    res
}

fn plugin_names(v: &ConfigVal) -> Vec<String> {
    let mut res = Vec::new();
    let items: Vec<ConfigVal> = match v {
        ConfigVal::Arr(a) => a.clone(),
        other => vec![other.clone()],
    };
    for it in items {
        match &it {
            ConfigVal::Obj(o) => res.extend(o.entries.iter().map(|(k, _)| k.clone())),
            _ => res.extend(as_str(&it)),
        }
    }
    res
}

/// A plugin name: a local file is an entry, a short name the
/// `@graphql-codegen/<name>` package, anything else a package.
fn plugin_tool(cmd: &Command, name: &str, out: &mut TaskSpec) {
    if name.starts_with('.') || name.starts_with('/') {
        let p = crate::recognizers::util::resolve_js_file(&cmd.cwd, name);
        out.entry(p, EntryLang::Js);
    } else if name.starts_with('@') || name.contains("codegen") {
        out.tool("npm", crate::recognizers::run::package_name(name));
    } else if name != "add" {
        out.tool("npm", format!("@graphql-codegen/{name}"));
    }
}

fn require(cmd: &Command, m: &str, out: &mut TaskSpec) {
    if m.starts_with('.') || m.starts_with('/') {
        let p = crate::recognizers::util::resolve_js_file(&cmd.cwd, m);
        out.entry(p, EntryLang::Js);
    } else {
        out.tool("npm", crate::recognizers::run::package_name(m));
    }
}
