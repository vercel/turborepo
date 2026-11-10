//! JS build tools: `tsc`, `next`, `vite`, `tsup`, `esbuild`, `rollup`,
//! `webpack`, and the linters/formatters that read whole directories (`eslint`,
//! `prettier`, `oxfmt`, `biome`, `stylelint`, `oxlint`).
//!
//! Configs written in code are configs *and* JS entries, so what they import
//! counts. Config values are read with the lang-js static config evaluator; a
//! value it cannot read leaves the config an input and adds an uncertainty plus
//! the package directory.

use std::path::{Path, PathBuf};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_frameworks::{fsx, nextjs::NextApp};
use turborepo_trace_lang_js::extract::config::ConfigVal;

use crate::{
    recognizers::{
        run::package_name,
        util::{
            Args, add_code_config, add_glob, add_path_input, find_config, find_up, flag,
            load_js_config, opt, opts, positionals, resolve_js_file, strings,
        },
    },
    registry::{Recognition, RecognizeCx, Recognizer},
    spec::{Command, EntryLang, Membership, Output, OutputKind, TaskPart, TaskSpec, Toolchain},
};

/// The recognizers of this module.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    vec![
        &Tsc, &Next, &Vite, &Tsup, &Esbuild, &Rollup, &Webpack, &Lint,
    ]
}

// ---------------------------------------------------------------------------------------
// tsconfig

/// A loaded tsconfig project (the `extends` chain merged).
#[derive(Clone, Debug, Default)]
pub struct TsProject {
    /// The project's tsconfig.
    pub file: PathBuf,
    /// Every config file read (the chain, referenced projects' configs).
    pub configs: Vec<PathBuf>,
    /// `include` and the directory it is relative to.
    pub include: Option<(PathBuf, Vec<String>)>,
    /// `exclude` and its directory.
    pub exclude: Option<(PathBuf, Vec<String>)>,
    /// `files` and their directory.
    pub files: Option<(PathBuf, Vec<String>)>,
    /// `references[].path`, resolved to tsconfig files.
    pub references: Vec<PathBuf>,
    /// `outDir`.
    pub out_dir: Option<PathBuf>,
    /// `declarationDir`.
    pub declaration_dir: Option<PathBuf>,
    /// `tsBuildInfoFile`.
    pub build_info: Option<PathBuf>,
    /// `noEmit` (the nearest config of the `extends` chain that sets it;
    /// `None`: unset).
    pub no_emit: Option<bool>,
    /// `emitDeclarationOnly` (as [`TsProject::no_emit`]).
    pub emit_declaration_only: Option<bool>,
    /// Packages providing a base config (`@tsconfig/node20`).
    pub packages: Vec<String>,
    /// Unreadable parts.
    pub problems: Vec<String>,
}

/// Resolves a tsconfig path argument: a file, or a directory holding
/// `tsconfig.json`.
pub fn tsconfig_path(p: PathBuf) -> PathBuf {
    if fsx::is_dir(&p) {
        p.join("tsconfig.json")
    } else {
        p
    }
}

fn resolve_extends(dir: &Path, spec: &str) -> Option<PathBuf> {
    if spec.starts_with('.') || spec.starts_with('/') {
        let p = fsx::join(dir, spec);
        if fsx::is_file(&p) {
            return Some(p);
        }
        let j = PathBuf::from(format!("{}.json", p.to_string_lossy()));
        if fsx::is_file(&j) {
            return Some(j);
        }
        return Some(p);
    }
    // A package: walk up node_modules.
    let mut d = Some(dir);
    while let Some(x) = d {
        let base = x.join("node_modules").join(spec);
        for cand in [
            base.clone(),
            PathBuf::from(format!("{}.json", base.to_string_lossy())),
            base.join("tsconfig.json"),
        ] {
            if fsx::is_file(&cand) {
                return Some(cand);
            }
        }
        d = x.parent();
    }
    None
}

/// Loads `file` and its `extends` chain (depth-limited).
pub fn load_tsconfig(file: &Path) -> TsProject {
    let mut p = TsProject {
        file: file.to_path_buf(),
        ..Default::default()
    };
    load_into(file, &mut p, 0);
    p
}

/// TypeScript 5.5's `${configDir}` template: a leading `${configDir}` stands
/// for the directory of the tsconfig being loaded (the leaf), even when it is
/// written in an extended base. Returns the absolute path string, or the value
/// unchanged.
fn expand_config_dir(s: &str, leaf_dir: &Path) -> String {
    match s.strip_prefix("${configDir}") {
        Some(rest) => {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                leaf_dir.to_string_lossy().into_owned()
            } else {
                leaf_dir.join(rest).to_string_lossy().into_owned()
            }
        }
        None => s.to_string(),
    }
}

fn load_into(file: &Path, p: &mut TsProject, depth: usize) {
    if depth > 16 || p.configs.iter().any(|c| c == file) {
        return;
    }
    p.configs.push(file.to_path_buf());
    let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let leaf_dir = p.file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let l = turborepo_trace_frameworks::jsconfig::load(
        file,
        turborepo_trace_frameworks::jsconfig::LoadOptions::default(),
    );
    let Some(v) = l.value else {
        if fsx::is_file(file) {
            p.problems
                .push(format!("unreadable tsconfig {}", file.display()));
        }
        return;
    };
    // Child values win: only set what is not set yet, then recurse into bases.
    let list = |key: &str, p: &mut TsProject| -> Option<(PathBuf, Vec<String>)> {
        let f = v.field(key)?;
        let (s, complete) = strings(&f);
        if !complete {
            p.problems
                .push(format!("`{key}` in {} is not static", file.display()));
        }
        let s = s.iter().map(|x| expand_config_dir(x, &leaf_dir)).collect();
        Some((dir.clone(), s))
    };
    if p.include.is_none() {
        p.include = list("include", p);
    }
    if p.exclude.is_none() {
        p.exclude = list("exclude", p);
    }
    if p.files.is_none() {
        p.files = list("files", p);
    }
    if depth == 0
        && let Some(ConfigVal::Arr(refs)) = v.field("references")
    {
        for r in refs {
            if let Some(s) = r
                .field("path")
                .as_ref()
                .and_then(|x| x.as_str().map(str::to_string))
            {
                p.references.push(tsconfig_path(fsx::join(&dir, &s)));
            }
        }
    }
    if let Some(co) = v.field("compilerOptions") {
        let path_opt = |k: &str| {
            co.field(k).and_then(|x| {
                x.as_str()
                    .map(|s| fsx::join(&dir, &expand_config_dir(s, &leaf_dir)))
            })
        };
        if p.out_dir.is_none() {
            p.out_dir = path_opt("outDir");
        }
        if p.declaration_dir.is_none() {
            p.declaration_dir = path_opt("declarationDir");
        }
        if p.build_info.is_none() {
            p.build_info = path_opt("tsBuildInfoFile");
        }
        // Inherited through `extends` like every compiler option (a shared base with
        // `"noEmit": true` makes every app that extends it a type-check-only project).
        if p.no_emit.is_none()
            && let Some(ConfigVal::Bool(b)) = co.field("noEmit")
        {
            p.no_emit = Some(b);
        }
        if p.emit_declaration_only.is_none()
            && let Some(ConfigVal::Bool(b)) = co.field("emitDeclarationOnly")
        {
            p.emit_declaration_only = Some(b);
        }
    }
    if let Some(ext) = v.field("extends") {
        let (bases, _) = strings(&ext);
        // `"extends": [a, b]`: later entries override earlier ones, so the last is read
        // first (values already set win).
        for b in bases.into_iter().rev() {
            if !(b.starts_with('.') || b.starts_with('/')) {
                p.packages.push(package_name(&b).to_string());
            }
            match resolve_extends(&dir, &b) {
                Some(f) => load_into(&f, p, depth + 1),
                None => p.problems.push(format!("tsconfig base `{b}` not found")),
            }
        }
    }
}

/// Projects one `tsc -b` walks at most; beyond, the task is marked incomplete
/// (a solution tsconfig in a large monorepo references 105 projects).
const MAX_TS_PROJECTS: usize = 1024;

/// What one tsconfig project contributes to a task: its inputs, and what `tsc`
/// writes for it.
struct TsUnit {
    /// The project's tsconfig.
    file: PathBuf,
    /// The config chain.
    configs: Vec<PathBuf>,
    /// Packages providing base configs.
    packages: Vec<String>,
    /// Unreadable parts.
    problems: Vec<String>,
    /// `files`, and `include` entries that name a file.
    inputs: Vec<PathBuf>,
    /// Whether the project emits JavaScript: importers of its output then run
    /// copies of its sources. Otherwise `tsc` only reads them (a
    /// type-check, declarations).
    emits_js: bool,
    /// `include` (minus `exclude` and the output directories).
    memberships: Vec<Membership>,
    /// What `tsc` emits for the project.
    outputs: Vec<Output>,
    /// `references`.
    references: Vec<PathBuf>,
    /// What reading the configuration observed (config reads, probes for config
    /// bases).
    observations: turborepo_trace_core::Observations,
}

/// Declaration files below an emit directory: `x.d.ts`, `x.d.mts`, `x.d.cts`
/// and their maps.
const DECLARATION_PATTERNS: &[&str] = &["**/*.d.*ts", "**/*.d.*ts.map"];

/// Reads the tsconfig project `file`.
///
/// Outputs follow what `tsc` emits. A `noEmit` project writes only its
/// `tsBuildInfoFile`. An `emitDeclarationOnly` project writes declarations
/// (`*.d.ts` and maps) into `outDir`, so a runtime import of `outDir/index.js`
/// is not its product. `exact_emit: false` (emit flags on the command line
/// override the config) keeps the whole emit directories instead.
fn ts_unit(file: &Path, exact_emit: bool) -> TsUnit {
    let (p, observations) = turborepo_trace_core::record::capture(|| load_tsconfig(file));
    // The enclosing frame (the task's) sees the same observations.
    turborepo_trace_core::record::replay(&observations);
    let cfg_dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut u = TsUnit {
        file: file.to_path_buf(),
        configs: p.configs.clone(),
        packages: p.packages.clone(),
        problems: p.problems.clone(),
        inputs: Vec::new(),
        emits_js: false,
        memberships: Vec::new(),
        outputs: Vec::new(),
        references: p.references.clone(),
        observations,
    };
    if let Some((d, files)) = &p.files {
        for f in files {
            u.inputs.push(fsx::join(d, f));
        }
    }
    let include = match (&p.include, &p.files) {
        (Some(i), _) => Some(i.clone()),
        (None, Some(_)) => None,
        (None, None) => Some((cfg_dir.clone(), vec!["**/*".to_string()])),
    };
    let mut excludes: Vec<(PathBuf, String)> = match &p.exclude {
        Some((d, e)) => e.iter().map(|x| (d.clone(), x.clone())).collect(),
        None => ["node_modules", "bower_components", "jspm_packages"]
            .iter()
            .map(|x| (cfg_dir.clone(), x.to_string()))
            .collect(),
    };
    // `tsc` never reads its own emit directories.
    if let Some(o) = &p.out_dir {
        excludes.push((o.clone(), String::new()));
    }
    if let Some(d) = p
        .declaration_dir
        .as_ref()
        .filter(|d| Some(*d) != p.out_dir.as_ref())
    {
        excludes.push((d.clone(), String::new()));
    }
    if let Some((d, pats)) = include {
        for pat in pats {
            let pat = if !crate::glob::is_glob(&pat) && Path::new(&pat).extension().is_none() {
                format!("{}/**/*", pat.trim_end_matches('/'))
            } else {
                pat
            };
            let (prefix, rest) = crate::glob::split_static_prefix(&pat);
            let dir = fsx::join(&d, prefix);
            if rest.is_empty() {
                u.inputs.push(dir);
                continue;
            }
            let mut m = Membership::new(dir.clone(), rest, "tsconfig include");
            for (ed, e) in &excludes {
                let abs = if e.is_empty() {
                    ed.clone()
                } else {
                    fsx::join(ed, e)
                };
                let ex = if let Ok(r) = abs.strip_prefix(&dir) {
                    let r = r.to_string_lossy();
                    if r.is_empty() {
                        continue;
                    }
                    if crate::glob::is_glob(&r) {
                        r.to_string()
                    } else {
                        format!("{r}/**")
                    }
                } else if e.starts_with("**") {
                    e.clone()
                } else {
                    continue;
                };
                m.exclude.push(ex);
            }
            u.memberships.push(m);
        }
    }
    let declarations = |dir: &Path, outputs: &mut Vec<Output>| {
        for pat in DECLARATION_PATTERNS {
            outputs.push(Output {
                pattern: Some((*pat).to_string()),
                ..Output::path(dir)
            });
        }
    };
    if !exact_emit || !p.no_emit.unwrap_or(false) {
        let declaration_only = exact_emit && p.emit_declaration_only.unwrap_or(false);
        if !declaration_only {
            // The project emits JavaScript: whoever imports the output runs copies of these
            // sources, so what they load at run time (their uncertainty) is the importer's.
            // A type-check or declaration-only project only reads them.
            u.emits_js = true;
        }
        if let Some(o) = &p.out_dir {
            if !declaration_only {
                u.outputs.push(Output::path(o.clone()));
            } else if p.declaration_dir.is_none() {
                declarations(o, &mut u.outputs);
            }
        }
        if let Some(d) = &p.declaration_dir {
            if exact_emit {
                declarations(d, &mut u.outputs);
            } else {
                u.outputs.push(Output::path(d.clone()));
            }
        }
    }
    if let Some(b) = &p.build_info {
        u.outputs.push(Output::path(b.clone()));
    }
    for m in &mut u.memberships {
        m.data = !u.emits_js;
    }
    u
}

impl TsUnit {
    /// Adds the project to `out`: configs, inputs, memberships and outputs.
    /// With `part`, the outputs are tagged with the project's [`TaskPart`].
    fn add_to(&self, out: &mut TaskSpec, part: bool) {
        for c in &self.configs {
            out.config(c.clone());
        }
        for pk in &self.packages {
            out.tool("npm", pk.clone());
        }
        for pr in &self.problems {
            out.uncertain(UncertaintyKind::ParseIncomplete, pr.clone());
        }
        // The files the project names (`files`, an `include` entry such as `build.ts`):
        // compiled like its other sources, or only read (see `emits_js`).
        if self.emits_js {
            out.inputs.extend(self.inputs.iter().cloned());
        } else {
            out.data_inputs.extend(self.inputs.iter().cloned());
        }
        out.memberships.extend(self.memberships.iter().cloned());
        out.outputs.extend(self.outputs.iter().map(|o| Output {
            part: part.then(|| self.file.clone()),
            ..o.clone()
        }));
    }
}

/// Reads the project `file` and, with `references`, every project it references
/// (transitively, each once, at most [`MAX_TS_PROJECTS`]) into `units`.
fn collect_ts_projects(
    file: &Path,
    references: bool,
    exact_emit: bool,
    out: &mut TaskSpec,
    seen: &mut Vec<PathBuf>,
    units: &mut Vec<TsUnit>,
) {
    if seen.iter().any(|s| s == file) {
        return;
    }
    if seen.len() >= MAX_TS_PROJECTS {
        // Dropping projects silently would drop their inputs: say so instead (once).
        if seen.len() == MAX_TS_PROJECTS {
            seen.push(PathBuf::new());
            out.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!(
                    "more than {MAX_TS_PROJECTS} tsconfig projects; {} and later not read",
                    file.display()
                ),
            );
        }
        return;
    }
    seen.push(file.to_path_buf());
    let unit = ts_unit(file, exact_emit);
    let refs = unit.references.clone();
    units.push(unit);
    if references {
        for r in &refs {
            collect_ts_projects(r, true, exact_emit, out, seen, units);
        }
    }
}

/// Adds a tsconfig project to `out`: configs, `files` inputs, `include`
/// memberships (minus `exclude`), outputs; referenced projects too when
/// `references` is true.
pub fn add_ts_project(file: &Path, references: bool, out: &mut TaskSpec, seen: &mut Vec<PathBuf>) {
    add_ts_projects(&[file.to_path_buf()], references, true, None, out, seen);
}

/// [`add_ts_project`] for several projects. With `part` (`tsc -b`: the part
/// kind and the npm package of the compiler), every project whose
/// configuration, and that of every project it references, was read completely
/// also becomes a [`TaskPart`] that owns the project's outputs: what `tsc -b`
/// writes for a project depends on that project and its references only,
/// whichever task runs it.
fn add_ts_projects(
    files: &[PathBuf],
    references: bool,
    exact_emit: bool,
    part: Option<(&str, &str)>,
    out: &mut TaskSpec,
    seen: &mut Vec<PathBuf>,
) {
    let mut units = Vec::new();
    for f in files {
        collect_ts_projects(f, references, exact_emit, out, seen, &mut units);
    }
    // A project is complete when it has no problem and every project it references
    // was read and is complete (to a fixed point: references may form cycles).
    let index: std::collections::HashMap<&Path, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.file.as_path(), i))
        .collect();
    let mut complete: Vec<bool> = units
        .iter()
        .map(|u| {
            part.is_some()
                && references
                && u.problems.is_empty()
                && u.references.iter().all(|r| index.contains_key(r.as_path()))
        })
        .collect();
    loop {
        let mut changed = false;
        for (i, u) in units.iter().enumerate() {
            if complete[i] && u.references.iter().any(|r| !complete[index[r.as_path()]]) {
                complete[i] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (i, u) in units.iter().enumerate() {
        u.add_to(out, complete[i]);
        if !references {
            for r in &u.references {
                out.config(r.clone());
            }
        }
        if let (true, Some((kind, tool))) = (complete[i], part) {
            let mut toolchain = vec![Toolchain::npm(tool)];
            toolchain.extend(u.packages.iter().map(Toolchain::npm));
            out.parts.push(TaskPart {
                kind: kind.to_string(),
                path: u.file.clone(),
                configs: u.configs.clone(),
                inputs: if u.emits_js {
                    u.inputs.clone()
                } else {
                    Vec::new()
                },
                data_inputs: if u.emits_js {
                    Vec::new()
                } else {
                    u.inputs.clone()
                },
                memberships: u.memberships.clone(),
                toolchain,
                deps: u.references.clone(),
                observations: u.observations.clone(),
            });
        }
    }
}

/// [`TaskPart::kind`] of a tsconfig project built by `tsc -b`.
pub const TS_PROJECT: &str = "tsproject";

/// `tsc [-p X] [--build X…] [files…]`.
pub struct Tsc;

impl Recognizer for Tsc {
    fn id(&self) -> &'static str {
        "tsc"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["tsc", "vue-tsc", "tsc-alias", "tspc", "ttsc", "tsgo"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("tsc");
        let tool = match prog {
            "tsc" => "typescript",
            "tsgo" => "@typescript/native-preview",
            p => p,
        };
        out.tool("npm", tool);
        let args = Args::new(
            cmd,
            0,
            &[
                "-p",
                "--project",
                "--outDir",
                "--declarationDir",
                "--rootDir",
                "--target",
                "--module",
                "--lib",
                "--jsx",
                "--moduleResolution",
                "--tsBuildInfoFile",
                "-t",
                "-m",
            ],
        )
        .collect_all();
        let build = flag(&args, &["-b", "--build"]);
        let outputs_before = out.outputs.len();
        let (memberships_before, inputs_before) = (out.memberships.len(), out.inputs.len());
        let pos = positionals(&args);
        let mut seen = Vec::new();
        // Emit options on the command line override the configs (`tsc` option names are
        // case-insensitive): the outputs are then not read from the configs exactly,
        // and not a function of the project alone.
        let exact_emit = !cmd.args().any(|a| {
            let a = a.split('=').next().unwrap_or(a).to_ascii_lowercase();
            matches!(
                a.as_str(),
                "--noemit"
                    | "--emitdeclarationonly"
                    | "--declaration"
                    | "-d"
                    | "--outdir"
                    | "--outfile"
                    | "--declarationdir"
                    | "--tsbuildinfofile"
                    | "--composite"
                    | "--incremental"
            )
        });
        if build {
            let projects: Vec<PathBuf> = if pos.is_empty() {
                vec![cmd.path("tsconfig.json")]
            } else {
                pos.iter().map(|p| tsconfig_path(cmd.path(p))).collect()
            };
            // The compiler is part of what a project's outputs depend on: one part kind per
            // compiler.
            let kind = if prog == "tsc" {
                TS_PROJECT.to_string()
            } else {
                format!("{TS_PROJECT}-{prog}")
            };
            let part = (exact_emit && prog != "tsc-alias").then_some((kind.as_str(), tool));
            add_ts_projects(&projects, true, exact_emit, part, out, &mut seen);
        } else if let Some(p) = opt(&args, &["-p", "--project"]) {
            add_ts_projects(
                &[tsconfig_path(cmd.path(p))],
                false,
                exact_emit,
                None,
                out,
                &mut seen,
            );
        } else if !pos.is_empty() {
            // Explicit files: tsconfig is ignored.
            for f in pos {
                if crate::glob::is_glob(f) {
                    add_glob(&cmd.cwd, f, "tsc files", out);
                } else {
                    out.entry(cmd.path(f), EntryLang::Js);
                }
            }
        } else {
            match find_up(&cmd.cwd, "tsconfig.json", Some(cx.repo_root)) {
                Some(t) => add_ts_projects(&[t], false, exact_emit, None, out, &mut seen),
                None => {
                    out.uncertain(UncertaintyKind::UnknownCommand, "tsc without a tsconfig");
                    cx.whole_package(out, "tsc without a tsconfig");
                    return Recognition::Partial;
                }
            }
        }
        if let Some(o) = opt(&args, &["--outDir"]) {
            out.output(cmd.path(o));
        }
        if let Some(o) = opt(&args, &["--declarationDir"]) {
            out.output(cmd.path(o));
        }
        if flag(&args, &["--noEmit"]) {
            out.outputs.truncate(outputs_before);
            // A type-check, whatever the configs say about emit: the projects' sources are
            // only read (the compiler variants that run transformer plugins aside).
            if matches!(prog, "tsc" | "tsgo" | "vue-tsc") {
                for m in &mut out.memberships[memberships_before..] {
                    m.data = true;
                }
                let read = out.inputs.split_off(inputs_before);
                out.data_inputs.extend(read);
            }
        }
        if prog == "tsc-alias" {
            // Rewrites emitted files in place.
            cx.whole_package(out, "tsc-alias rewrites build output");
        }
        if out
            .uncertainties
            .iter()
            .any(|u| u.kind == UncertaintyKind::ParseIncomplete)
        {
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

// ---------------------------------------------------------------------------------------
// next

/// `next build|dev|start|lint|export [dir]`.
pub struct Next;

impl Recognizer for Next {
    fn id(&self) -> &'static str {
        "next"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["next"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "next");
        let args = Args::new(
            cmd,
            0,
            &[
                "-p",
                "--port",
                "-H",
                "--hostname",
                "-o",
                "--outdir",
                "--dir",
                "-d",
                "--experimental-https-key",
                "--experimental-https-cert",
                "--keepAliveTimeout",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let (sub, rest): (&str, &[&str]) = match pos.split_first() {
            Some((s, r))
                if matches!(
                    *s,
                    "build"
                        | "dev"
                        | "start"
                        | "lint"
                        | "export"
                        | "info"
                        | "telemetry"
                        | "typegen"
                        | "experimental-analyze"
                ) =>
            {
                (s, r)
            }
            _ => ("dev", &pos[..]),
        };
        let root = match rest.first() {
            Some(d) => cmd.path(d),
            None => cmd.cwd.clone(),
        };
        let app = NextApp::load(&root);
        if let Some(f) = &app.config.file {
            add_code_config(f, out);
        }
        for f in &app.config.files {
            out.config(f.clone());
        }
        for f in &app.config.code_files {
            out.entry(f.clone(), EntryLang::Js);
        }
        for u in &app.config.uncertainties {
            out.uncertain(u.kind, format!("next.config: {}", u.detail));
        }
        match sub {
            "start" => {
                out.input(root.join(".next"));
                Recognition::Full
            }
            "export" => {
                out.input(root.join(".next"));
                out.output(match opt(&args, &["-o", "--outdir"]) {
                    Some(o) => cmd.path(o),
                    None => root.join("out"),
                });
                Recognition::Full
            }
            "lint" => {
                let dirs = opts(&args, &["--dir", "-d"]);
                if dirs.is_empty() {
                    for d in ["app", "src", "pages", "components", "lib"] {
                        out.membership(root.join(d), "**", "next lint default dirs");
                    }
                } else {
                    for d in dirs {
                        out.membership(cmd.path(d), "**", "next lint --dir");
                    }
                }
                eslint_configs(&root, cx.repo_root, out);
                Recognition::Full
            }
            "info" | "telemetry" => Recognition::Ignored,
            _ => {
                // build, dev, typegen: every route file and the global inputs.
                let fi = app.framework_inputs();
                for proj in &fi.projects {
                    for f in &proj.global.config_files {
                        out.config(f.clone());
                    }
                    for f in &proj.global.js_entries {
                        out.entry(f.clone(), EntryLang::Js);
                    }
                    for p in &proj.global.probes {
                        out.input(p.clone());
                    }
                    for m in &proj.global.memberships {
                        out.membership(m.dir.clone(), m.pattern.clone(), m.reason.clone());
                    }
                }
                for r in &app.table.routes {
                    out.entry(root.join(&r.file), EntryLang::Js);
                }
                for d in &app.table.dirs {
                    for f in d.files.values() {
                        out.entry(root.join(f), EntryLang::Js);
                    }
                }
                for f in &app.table.pages_special {
                    out.entry(root.join(f), EntryLang::Js);
                }
                // `next build` type-checks the tsconfig project.
                let ts = root.join("tsconfig.json");
                if fsx::is_file(&ts) {
                    add_ts_project(&ts, false, out, &mut Vec::new());
                }
                if sub == "build" {
                    out.output(root.join(".next"));
                }
                for k in ["NODE_ENV", "NEXT_PUBLIC_*"] {
                    out.env.push(k.to_string());
                }
                if app.config.file.is_none()
                    && app.table.app_dir.is_none()
                    && app.table.pages_dir.is_none()
                {
                    out.uncertain(UncertaintyKind::UnknownCommand, "next: no app found");
                    cx.whole_package(out, "next: no app found");
                    return Recognition::Partial;
                }
                Recognition::Full
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// vite

const VITE_CONFIGS: &[&str] = &[
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.ts",
    "vite.config.cjs",
    "vite.config.mts",
    "vite.config.cts",
];

/// `vite [build|dev|serve|preview] [root]`.
pub struct Vite;

impl Recognizer for Vite {
    fn id(&self) -> &'static str {
        "vite"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["vite"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "vite");
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "--base",
                "-m",
                "--mode",
                "--outDir",
                "--port",
                "--host",
                "-l",
                "--logLevel",
                "--assetsDir",
                "--target",
                "--envDir",
            ],
        )
        .collect_all();
        let pos = positionals(&args);
        let (sub, rest): (&str, &[&str]) = match pos.split_first() {
            Some((s, r)) if matches!(*s, "build" | "dev" | "serve" | "preview" | "optimize") => {
                (s, r)
            }
            _ => ("dev", &pos[..]),
        };
        let mut root = match rest.first() {
            Some(d) => cmd.path(d),
            None => cmd.cwd.clone(),
        };
        let config = match opt(&args, &["-c", "--config"]) {
            Some(c) => Some(cmd.path(c)),
            None => find_config(&root, VITE_CONFIGS),
        };
        let mut partial = false;
        let mut out_dir: Option<PathBuf> = None;
        let mut inputs_from_config: Vec<String> = Vec::new();
        let mut public_dir: Option<PathBuf> = None;
        if let Some(c) = &config {
            let cfg_dir = c.parent().unwrap_or(&root).to_path_buf();
            match load_js_config(c, out) {
                Some(v) if !matches!(v, ConfigVal::Function(_) | ConfigVal::Unknown(_)) => {
                    if let Some(r) = v
                        .field("root")
                        .as_ref()
                        .and_then(|x| x.as_str().map(str::to_string))
                    {
                        root = fsx::join(&cfg_dir, &r);
                    }
                    if let Some(b) = v.field("build") {
                        if let Some(o) = b
                            .field("outDir")
                            .as_ref()
                            .and_then(|x| x.as_str().map(str::to_string))
                        {
                            out_dir = Some(fsx::join(&root, &o));
                        }
                        if let Some(i) = b.field("rollupOptions").and_then(|r| r.field("input")) {
                            let (s, complete) = strings(&i);
                            partial |= !complete;
                            inputs_from_config.extend(s);
                        }
                        if let Some(e) = b.field("lib").and_then(|l| l.field("entry")) {
                            let (s, complete) = strings(&e);
                            partial |= !complete;
                            inputs_from_config.extend(s);
                        }
                    }
                    if let Some(p) = v.field("publicDir") {
                        match p.as_str() {
                            Some(s) => public_dir = Some(fsx::join(&root, s)),
                            None if matches!(p, ConfigVal::Bool(false)) => {}
                            None => partial = true,
                        }
                    }
                    if v.has_unknown() {
                        partial = true;
                    }
                }
                _ => partial = true,
            }
        }
        if let Some(o) = opt(&args, &["--outDir"]) {
            out_dir = Some(cmd.path(o));
        }
        if sub == "preview" {
            out.input(out_dir.unwrap_or_else(|| root.join("dist")));
            return Recognition::Full;
        }
        if inputs_from_config.is_empty() {
            let index = root.join("index.html");
            html_entry(&index, &root, out);
        } else {
            for i in &inputs_from_config {
                let p = fsx::join(&root, i);
                if p.extension().is_some_and(|e| e == "html") {
                    html_entry(&p, &root, out);
                } else {
                    out.entry(resolve_js_file(&root, i), EntryLang::Js);
                }
            }
        }
        out.membership(
            public_dir.unwrap_or_else(|| root.join("public")),
            "**",
            "vite publicDir",
        );
        let env_dir = opt(&args, &["--envDir"])
            .map(|d| cmd.path(d))
            .unwrap_or_else(|| root.clone());
        out.membership(env_dir, ".env*", "vite env files");
        if sub == "build" {
            out.output(out_dir.unwrap_or_else(|| root.join("dist")));
        }
        if partial {
            out.uncertain(
                UncertaintyKind::ParseIncomplete,
                "vite config not fully static",
            );
            out.membership(root, "**", "vite config not fully static");
            let _ = cx;
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// An HTML entry: the file is an input, and its local `<script src>` / `<link
/// href>` targets are entries (absolute URLs are relative to `root`).
pub fn html_entry(html: &Path, root: &Path, out: &mut TaskSpec) {
    out.input(html.to_path_buf());
    let Ok(src) = fsx::read_to_string(html) else {
        return;
    };
    let dir = html.parent().unwrap_or(root);
    for attr in ["src=", "href="] {
        let mut rest = src.as_str();
        while let Some(i) = rest.find(attr) {
            rest = &rest[i + attr.len()..];
            let q = rest.chars().next().unwrap_or(' ');
            if q != '"' && q != '\'' {
                continue;
            }
            let Some(end) = rest[1..].find(q) else { break };
            let v = &rest[1..1 + end];
            rest = &rest[1 + end..];
            if v.contains("://")
                || v.starts_with("//")
                || v.starts_with('#')
                || v.starts_with("data:")
                || v.is_empty()
            {
                continue;
            }
            let v = v.split(['?', '#']).next().unwrap_or(v);
            let p = if let Some(abs) = v.strip_prefix('/') {
                root.join(abs)
            } else {
                fsx::join(dir, v)
            };
            let p = fsx::normalize(&p);
            if EntryLang::of(&p) == EntryLang::Js {
                out.entry(p, EntryLang::Js);
            } else if fsx::is_file(&p) || attr == "src=" {
                out.input(p);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// tsup, esbuild, rollup, webpack

const TSUP_CONFIGS: &[&str] = &[
    "tsup.config.ts",
    "tsup.config.cts",
    "tsup.config.mts",
    "tsup.config.js",
    "tsup.config.cjs",
    "tsup.config.mjs",
    "tsup.config.json",
];

/// `tsup [entries…] [--config f] [-d dir]`.
pub struct Tsup;

impl Recognizer for Tsup {
    fn id(&self) -> &'static str {
        "tsup"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["tsup", "tsup-node", "tsdown"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("tsup");
        out.tool("npm", if prog == "tsup-node" { "tsup" } else { prog });
        let args = Args::new(
            cmd,
            0,
            &[
                "--config",
                "-c",
                "-d",
                "--out-dir",
                "--format",
                "--target",
                "--tsconfig",
                "--env",
                "--external",
                "--platform",
                "--onSuccess",
                "--on-success",
                "--inject",
                "--loader",
                "--entry",
            ],
        )
        .collect_all();
        let mut entries: Vec<String> = positionals(&args).iter().map(|s| s.to_string()).collect();
        entries.extend(opts(&args, &["--entry"]).into_iter().map(str::to_string));
        let mut partial = false;
        let mut out_dir = None;
        let config = match opt(&args, &["--config", "-c"]) {
            Some(c) => Some(cmd.path(c)),
            None if flag(&args, &["--no-config"]) => None,
            None => find_config(
                &cmd.cwd,
                if prog == "tsdown" {
                    &["tsdown.config.ts", "tsdown.config.js", "tsdown.config.mjs"]
                } else {
                    TSUP_CONFIGS
                },
            ),
        };
        let config_val = match &config {
            Some(c) => {
                let v = load_js_config(c, out);
                if v.is_none() {
                    partial = true;
                }
                v
            }
            None => {
                let pj = cmd.cwd.join("package.json");
                let v = crate::sources::read_json(&pj).and_then(|j| j.get(prog).cloned());
                v.map(|j| json_to_config(&j))
            }
        };
        if let Some(v) = config_val {
            for cfg in match &v {
                ConfigVal::Arr(a) => a.clone(),
                other => vec![other.clone()],
            } {
                if matches!(cfg, ConfigVal::Function(_) | ConfigVal::Unknown(_)) {
                    partial = true;
                    continue;
                }
                if entries.is_empty()
                    && let Some(e) = cfg.field("entry").or_else(|| cfg.field("entryPoints"))
                {
                    let (s, complete) = strings(&e);
                    partial |= !complete;
                    entries.extend(s);
                }
                if let Some(o) = cfg
                    .field("outDir")
                    .as_ref()
                    .and_then(|x| x.as_str().map(str::to_string))
                {
                    out_dir = Some(cmd.path(&o));
                }
                if let Some(t) = cfg
                    .field("tsconfig")
                    .as_ref()
                    .and_then(|x| x.as_str().map(str::to_string))
                {
                    out.config(cmd.path(&t));
                }
            }
        }
        if entries.is_empty() {
            entries.push("src/index.ts".into());
        }
        for e in &entries {
            if crate::glob::is_glob(e) {
                add_glob(&cmd.cwd, e, "tsup entry glob", out);
            } else {
                out.entry(resolve_js_file(&cmd.cwd, e), EntryLang::Js);
            }
        }
        match opt(&args, &["--tsconfig"]) {
            Some(t) => out.config(cmd.path(t)),
            None => {
                if let Some(t) = find_up(&cmd.cwd, "tsconfig.json", Some(cx.repo_root)) {
                    out.config(t);
                }
            }
        }
        if flag(&args, &["--dts", "--dts-only", "--experimental-dts"]) {
            let ts = cmd.cwd.join("tsconfig.json");
            if fsx::is_file(&ts) {
                add_ts_project(&ts, false, out, &mut Vec::new());
            }
        }
        out.output(
            opt(&args, &["-d", "--out-dir"])
                .map(|d| cmd.path(d))
                .or(out_dir)
                .unwrap_or_else(|| cmd.path("dist")),
        );
        if partial {
            out.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!("{prog} config not fully static"),
            );
            cx.whole_package(out, &format!("{prog} config not fully static"));
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// A JSON value as a config value.
pub fn json_to_config(j: &serde_json::Value) -> ConfigVal {
    use serde_json::Value as J;
    match j {
        J::Null => ConfigVal::Null,
        J::Bool(b) => ConfigVal::Bool(*b),
        J::Number(n) => ConfigVal::Num(n.to_string()),
        J::String(s) => ConfigVal::str(s.clone()),
        J::Array(a) => ConfigVal::Arr(a.iter().map(json_to_config).collect()),
        J::Object(o) => {
            let mut obj = turborepo_trace_lang_js::extract::config::ConfigObj::default();
            for (k, v) in o {
                obj.set(k.clone(), json_to_config(v));
            }
            ConfigVal::Obj(obj)
        }
    }
}

/// `esbuild entries… [--outfile=x|--outdir=x] [--tsconfig=x] [--inject:x]`.
pub struct Esbuild;

impl Recognizer for Esbuild {
    fn id(&self) -> &'static str {
        "esbuild"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["esbuild"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "esbuild");
        let mut any = false;
        let mut partial = false;
        for w in cmd.arg_words() {
            let a = w.text.as_str();
            if let Some(opt) = a.strip_prefix("--") {
                let (k, v) = match opt.split_once('=') {
                    Some((k, v)) => (k, Some(v)),
                    None => match opt.split_once(':') {
                        Some((k, v)) => (k, Some(v)),
                        None => (opt, None),
                    },
                };
                match (k, v) {
                    ("outfile" | "outdir" | "metafile", Some(v)) => out.output(cmd.path(v)),
                    ("tsconfig", Some(v)) => out.config(cmd.path(v)),
                    ("inject", Some(v)) => out.entry(resolve_js_file(&cmd.cwd, v), EntryLang::Js),
                    ("alias", Some(_)) | ("external", Some(_)) => {}
                    _ => {}
                }
                continue;
            }
            if a.starts_with('-') {
                continue;
            }
            any = true;
            if w.dynamic {
                partial = true;
                continue;
            }
            // `out=in` entry syntax.
            let src = a.split_once('=').map_or(a, |(_, s)| s);
            if w.glob || crate::glob::is_glob(src) {
                add_glob(&cmd.cwd, src, "esbuild entry glob", out);
            } else {
                out.entry(cmd.path(src), EntryLang::of(Path::new(src)));
            }
        }
        if let Some(t) = find_up(&cmd.cwd, "tsconfig.json", Some(cx.repo_root)) {
            out.config(t);
        }
        if !any || partial {
            out.uncertain(
                UncertaintyKind::UnknownCommand,
                "esbuild without static entry points",
            );
            cx.whole_package(out, "esbuild without static entry points");
            return Recognition::Partial;
        }
        Recognition::Full
    }
}

const ROLLUP_CONFIGS: &[&str] = &[
    "rollup.config.js",
    "rollup.config.mjs",
    "rollup.config.cjs",
    "rollup.config.ts",
    "rollup.config.mts",
    "rollup.config.cts",
];

/// `rollup -c [file]` / `rollup -i x -o y`.
pub struct Rollup;

impl Recognizer for Rollup {
    fn id(&self) -> &'static str {
        "rollup"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["rollup"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        out.tool("npm", "rollup");
        let words: Vec<&str> = cmd.args().collect();
        let mut config: Option<PathBuf> = None;
        let mut use_config = false;
        let mut inputs = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            i += 1;
            let (k, inline) = match w.split_once('=') {
                Some((k, v)) if k.starts_with('-') => (k, Some(v)),
                _ => (w, None),
            };
            match k {
                "-c" | "--config" => {
                    use_config = true;
                    let v = inline.or_else(|| {
                        words
                            .get(i)
                            .copied()
                            .filter(|n| !n.starts_with('-') && (n.contains('.') || n.contains('/')))
                    });
                    if let Some(v) = v {
                        if inline.is_none() {
                            i += 1;
                        }
                        // `-c node:pkg` loads a package config.
                        if let Some(pkg) = v.strip_prefix("node:") {
                            out.tool("npm", pkg);
                        } else {
                            config = Some(cmd.path(v));
                        }
                    }
                }
                "-i" | "--input" => {
                    if let Some(v) = inline.or_else(|| words.get(i).copied()) {
                        if inline.is_none() {
                            i += 1;
                        }
                        inputs.push(v.to_string());
                    }
                }
                "-o" | "--file" | "-d" | "--dir" => {
                    if let Some(v) = inline.or_else(|| words.get(i).copied()) {
                        if inline.is_none() {
                            i += 1;
                        }
                        out.output(cmd.path(v));
                    }
                }
                "-p" | "--plugin" | "-f" | "--format" | "-n" | "--name" | "-g" | "--globals"
                | "-e" | "--external" | "--environment" => {
                    if inline.is_none() {
                        i += 1;
                    }
                }
                p if !p.starts_with('-') => inputs.push(p.to_string()),
                _ => {}
            }
        }
        let mut partial = false;
        if use_config {
            let c = config.or_else(|| find_config(&cmd.cwd, ROLLUP_CONFIGS));
            match c {
                Some(c) => match load_js_config(&c, out) {
                    Some(v) => {
                        let cfgs = match &v {
                            ConfigVal::Arr(a) => a.clone(),
                            other => vec![other.clone()],
                        };
                        let dir = c.parent().unwrap_or(&cmd.cwd).to_path_buf();
                        for cfg in cfgs {
                            if matches!(cfg, ConfigVal::Function(_) | ConfigVal::Unknown(_)) {
                                partial = true;
                                continue;
                            }
                            match cfg.field("input") {
                                Some(i) => {
                                    let (s, complete) = strings(&i);
                                    partial |= !complete;
                                    for x in s {
                                        out.entry(resolve_js_file(&dir, &x), EntryLang::Js);
                                    }
                                }
                                None => partial = true,
                            }
                            if let Some(o) = cfg.field("output") {
                                for oc in match &o {
                                    ConfigVal::Arr(a) => a.clone(),
                                    other => vec![other.clone()],
                                } {
                                    for k in ["file", "dir"] {
                                        if let Some(s) = oc
                                            .field(k)
                                            .as_ref()
                                            .and_then(|x| x.as_str().map(str::to_string))
                                        {
                                            out.output(fsx::join(&dir, &s));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None => partial = true,
                },
                None => partial = true,
            }
        }
        for x in inputs {
            out.entry(resolve_js_file(&cmd.cwd, &x), EntryLang::Js);
        }
        if partial {
            out.uncertain(
                UncertaintyKind::ParseIncomplete,
                "rollup config not fully static",
            );
            cx.whole_package(out, "rollup config not fully static");
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

const WEBPACK_CONFIGS: &[&str] = &[
    "webpack.config.js",
    "webpack.config.ts",
    "webpack.config.cjs",
    "webpack.config.mjs",
    "webpackfile.js",
    ".webpack/webpack.config.js",
];

/// `webpack [build|serve|watch] [--config f] [--entry x] [-o dir]`.
pub struct Webpack;

impl Recognizer for Webpack {
    fn id(&self) -> &'static str {
        "webpack"
    }
    fn programs(&self) -> &'static [&'static str] {
        &["webpack", "webpack-cli", "webpack-dev-server", "rspack"]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("webpack");
        out.tool(
            "npm",
            if prog == "rspack" {
                "@rspack/cli"
            } else {
                "webpack"
            },
        );
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "--entry",
                "-o",
                "--output-path",
                "--mode",
                "--env",
                "--config-name",
                "--port",
                "--target",
                "-t",
                "--devtool",
                "-d",
            ],
        )
        .collect_all();
        let mut partial = false;
        let configs: Vec<PathBuf> = {
            let c = opts(&args, &["-c", "--config"]);
            if c.is_empty() {
                let names: Vec<String> = if prog == "rspack" {
                    vec![
                        "rspack.config.js".into(),
                        "rspack.config.ts".into(),
                        "rspack.config.mjs".into(),
                    ]
                } else {
                    WEBPACK_CONFIGS.iter().map(|s| s.to_string()).collect()
                };
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                find_config(&cmd.cwd, &names).into_iter().collect()
            } else {
                c.into_iter().map(|c| cmd.path(c)).collect()
            }
        };
        let mut entries: Vec<String> = opts(&args, &["--entry"])
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut context = cmd.cwd.clone();
        for c in &configs {
            match load_js_config(c, out) {
                Some(v) => {
                    for cfg in match &v {
                        ConfigVal::Arr(a) => a.clone(),
                        other => vec![other.clone()],
                    } {
                        if matches!(cfg, ConfigVal::Function(_) | ConfigVal::Unknown(_)) {
                            partial = true;
                            continue;
                        }
                        if let Some(ctx) = cfg
                            .field("context")
                            .as_ref()
                            .and_then(|x| x.as_str().map(str::to_string))
                        {
                            context = fsx::join(&cmd.cwd, &ctx);
                        }
                        match cfg.field("entry") {
                            Some(e) => {
                                let mut vals = Vec::new();
                                entry_values(&e, &mut vals, &mut partial);
                                entries.extend(vals);
                            }
                            None => entries.push("./src".into()),
                        }
                        if let Some(p) = cfg
                            .field("output")
                            .and_then(|o| o.field("path"))
                            .as_ref()
                            .and_then(|x| x.as_str().map(str::to_string))
                        {
                            out.output(fsx::join(&cmd.cwd, &p));
                        }
                    }
                }
                None => partial = true,
            }
        }
        if configs.is_empty() && entries.is_empty() {
            entries.push("./src".into());
        }
        for e in &entries {
            if e.starts_with('.') || e.starts_with('/') {
                out.entry(resolve_js_file(&context, e), EntryLang::Js);
            } else {
                out.tool("npm", package_name(e));
            }
        }
        match opt(&args, &["-o", "--output-path"]) {
            Some(o) => out.output(cmd.path(o)),
            None if configs.is_empty() => out.output(cmd.path("dist")),
            None => {}
        }
        if partial {
            out.uncertain(
                UncertaintyKind::ParseIncomplete,
                format!("{prog} config not fully static"),
            );
            cx.whole_package(out, &format!("{prog} config not fully static"));
            Recognition::Partial
        } else {
            Recognition::Full
        }
    }
}

/// webpack `entry`: a string, array, object of strings/arrays/`{ import }`
/// descriptors.
fn entry_values(v: &ConfigVal, out: &mut Vec<String>, partial: &mut bool) {
    match v {
        ConfigVal::Obj(o) => {
            for (_, x) in &o.entries {
                match x.field("import") {
                    Some(i) if matches!(x, ConfigVal::Obj(_)) => entry_values(&i, out, partial),
                    _ => entry_values(x, out, partial),
                }
            }
            if o.open.is_some() {
                *partial = true;
            }
        }
        ConfigVal::Arr(a) | ConfigVal::Alts(a) => {
            a.iter().for_each(|x| entry_values(x, out, partial))
        }
        other => match other.as_str() {
            Some(s) => out.push(s.to_string()),
            None => *partial = true,
        },
    }
}

// ---------------------------------------------------------------------------------------
// Linters and formatters

const ESLINT_CONFIGS: &[&str] = &[
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    "eslint.config.mts",
    "eslint.config.cts",
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.json",
    ".eslintrc.yml",
    ".eslintrc.yaml",
    ".eslintignore",
];

const PRETTIER_CONFIGS: &[&str] = &[
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yml",
    ".prettierrc.yaml",
    ".prettierrc.json5",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    ".prettierrc.toml",
    "prettier.config.js",
    "prettier.config.cjs",
    "prettier.config.mjs",
    "prettier.config.ts",
    ".prettierignore",
    ".editorconfig",
];

fn eslint_configs(dir: &Path, repo_root: &Path, out: &mut TaskSpec) {
    let mut d = Some(dir);
    while let Some(x) = d {
        for n in ESLINT_CONFIGS {
            let p = x.join(n);
            if fsx::is_file(&p) {
                add_code_config(&p, out);
            }
        }
        if x == repo_root {
            break;
        }
        d = x.parent();
    }
}

/// `eslint`, `prettier`, `oxfmt`, `biome`, `stylelint`, `oxlint`, `next lint`
/// style tools: path arguments (default: the cwd) are memberships; config files
/// up the tree are configs.
pub struct Lint;

impl Recognizer for Lint {
    fn id(&self) -> &'static str {
        "lint"
    }
    fn programs(&self) -> &'static [&'static str] {
        &[
            "eslint",
            "prettier",
            "oxfmt",
            "biome",
            "stylelint",
            "oxlint",
            "markdownlint",
            "markdownlint-cli2",
            "xo",
            "standard",
            "dprint",
            "knip",
            "publint",
            "attw",
            "sort-package-json",
            "syncpack",
            "manypkg",
            "depcheck",
            "cspell",
            "lint-staged",
        ]
    }
    fn recognize(&self, cmd: &Command, cx: &RecognizeCx<'_>, out: &mut TaskSpec) -> Recognition {
        let prog = cmd.program_name().unwrap_or("");
        out.tool(
            "npm",
            match prog {
                "biome" => "@biomejs/biome",
                "markdownlint" => "markdownlint-cli",
                "attw" => "@arethetypeswrong/cli",
                "manypkg" => "@manypkg/cli",
                p => p,
            },
        );
        if prog == "lint-staged" {
            // Operates on the git index.
            out.uncertain(
                UncertaintyKind::EnvDependent,
                "lint-staged reads the git index",
            );
            return Recognition::Partial;
        }
        let args = Args::new(
            cmd,
            0,
            &[
                "-c",
                "--config",
                "--ext",
                "--ignore-path",
                "--format",
                "-f",
                "-o",
                "--output-file",
                "--rule",
                "--plugin",
                "--parser",
                "--resolve-plugins-relative-to",
                "--rulesdir",
                "--max-warnings",
                "--cache-location",
                "--log-level",
                "--loglevel",
                "--config-path",
                "--reporter",
                "--formatter",
                "--stdin-file-path",
                "--stdin-filepath",
            ],
        )
        .collect_all();
        if let Some(c) = opt(&args, &["-c", "--config", "--config-path"]) {
            add_code_config(&cmd.path(c), out);
        }
        if let Some(c) = opt(&args, &["--ignore-path"]) {
            out.config(cmd.path(c));
        }
        if let Some(o) = opt(&args, &["-o", "--output-file"]) {
            out.output(cmd.path(o));
        }
        match prog {
            "eslint" | "xo" | "standard" | "oxlint" => eslint_configs(&cmd.cwd, cx.repo_root, out),
            "prettier" => {
                let mut d = Some(cmd.cwd.as_path());
                while let Some(x) = d {
                    for n in PRETTIER_CONFIGS {
                        let p = x.join(n);
                        if fsx::is_file(&p) {
                            add_code_config(&p, out);
                        }
                    }
                    if x == cx.repo_root {
                        break;
                    }
                    d = x.parent();
                }
            }
            "biome" => {
                for n in ["biome.json", "biome.jsonc"] {
                    if let Some(p) = find_up(&cmd.cwd, n, Some(cx.repo_root)) {
                        out.config(p);
                    }
                }
            }
            "oxfmt" => {
                for n in [".oxfmtrc.json", ".oxfmtrc.jsonc"] {
                    if let Some(p) = find_up(&cmd.cwd, n, Some(cx.repo_root)) {
                        out.config(p);
                    }
                }
            }
            "stylelint" => {
                for n in [
                    ".stylelintrc",
                    ".stylelintrc.json",
                    ".stylelintrc.js",
                    ".stylelintrc.cjs",
                    "stylelint.config.js",
                    "stylelint.config.cjs",
                    "stylelint.config.mjs",
                    ".stylelintignore",
                ] {
                    if let Some(p) = find_up(&cmd.cwd, n, Some(cx.repo_root)) {
                        add_code_config(&p, out);
                    }
                }
            }
            _ => {}
        }
        out.input(cmd.cwd.join("package.json"));
        let mut pos = positionals(&args);
        // `biome check|lint|format|ci`, `markdownlint-cli2 …`, `dprint check|fmt`.
        if matches!(prog, "biome" | "dprint")
            && let Some(first) = pos.first()
            && !first.contains('.')
            && !first.contains('/')
        {
            pos.remove(0);
        }
        if matches!(
            prog,
            "knip" | "publint" | "attw" | "sort-package-json" | "syncpack" | "manypkg" | "depcheck"
        ) || pos.is_empty()
        {
            out.membership(cmd.cwd.clone(), "**", format!("{prog} default files"));
        }
        if lint_writes(prog, cmd) {
            // Formatters and `--fix` rewrite the files they check in place: recorded, but
            // never a producer (`OutputKind::InPlace`).
            let mut targets: Vec<&str> = pos
                .iter()
                .copied()
                .filter(|p| !cmd.argv.iter().any(|w| w.text == *p && w.dynamic))
                .collect();
            if targets.is_empty() {
                targets.push(if prog == "sort-package-json" {
                    "package.json"
                } else {
                    "."
                });
            }
            for t in targets {
                let abs = cmd.path(t).to_string_lossy().into_owned();
                let (prefix, rest) = crate::glob::split_static_prefix(&abs);
                out.outputs.push(Output {
                    path: fsx::normalize(Path::new(&format!(
                        "/{}",
                        prefix.trim_start_matches('/')
                    ))),
                    pattern: (!rest.is_empty()).then(|| rest.to_string()),
                    exclude: Vec::new(),
                    kind: OutputKind::InPlace,
                    part: None,
                });
            }
        }
        for p in pos {
            if cmd.argv.iter().any(|w| w.text == p && w.dynamic) {
                continue;
            }
            add_path_input(cmd, p, &format!("{prog} files"), out);
        }
        Recognition::Full
    }
}

/// Whether a [`Lint`] tool rewrites its files: `prettier --write`, `oxfmt`
/// without `--check`, `eslint --fix`, `biome format --write`, `dprint fmt`,
/// `sort-package-json` without `--check`.
fn lint_writes(prog: &str, cmd: &Command) -> bool {
    let has = |flags: &[&str]| cmd.args().any(|a| flags.contains(&a));
    match prog {
        "prettier" => has(&["--write", "-w"]),
        // oxfmt writes unless told to check.
        "oxfmt" => !has(&["--check", "--list-different"]),
        "eslint" | "xo" | "standard" | "oxlint" | "stylelint" | "markdownlint"
        | "markdownlint-cli2" => has(&["--fix"]),
        "biome" => has(&["--write", "--apply", "--apply-unsafe", "--fix"]),
        "dprint" => cmd.args().next() == Some("fmt"),
        "sort-package-json" => !has(&["--check", "-c"]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "turbotrace-js-{name}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (p, c) in files {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, c).unwrap();
        }
        root.canonicalize().unwrap()
    }

    const BASE: &str = r#"{
  "compilerOptions": {
    "composite": true,
    "outDir": "${configDir}/dist",
    "tsBuildInfoFile": "${configDir}/dist/tsbuildinfo.json"
  },
  "exclude": ["${configDir}/node_modules", "${configDir}/dist"]
}"#;

    /// Seen in a large monorepo: `@acme/typescript-config/package-bundler.json`
    /// writes `"outDir": "${configDir}/dist"` and excludes
    /// `${configDir}/node_modules`. Left literal, the outputs landed in
    /// `<pkg>/node_modules/.../${configDir}/dist` and the excludes never
    /// matched, so the `**/*` membership covered `node_modules` and `dist`, and
    /// `tsc -b` became the producer of the whole package for every test
    /// importing it.
    #[test]
    fn config_dir_template_resolves_to_the_leaf_tsconfig() {
        for (name, extends, base_path) in [
            ("rel", "../../tooling/ts/pkg.json", "tooling/ts/pkg.json"),
            (
                "pkg",
                "@acme/ts-config/pkg.json",
                "packages/jwt/node_modules/@acme/ts-config/pkg.json",
            ),
        ] {
            let leaf = format!(r#"{{ "extends": "{extends}" }}"#);
            let root = tree(
                name,
                &[
                    ("packages/jwt/tsconfig.json", leaf.as_str()),
                    ("packages/jwt/src/index.ts", "export {}"),
                    (base_path, BASE),
                ],
            );
            let pkg = root.join("packages/jwt");
            let mut out = TaskSpec::default();
            add_ts_project(&pkg.join("tsconfig.json"), true, &mut out, &mut Vec::new());
            let outs: Vec<&Path> = out.outputs.iter().map(|o| o.path.as_path()).collect();
            assert!(
                outs.contains(&pkg.join("dist").as_path()),
                "{name}: {outs:?}"
            );
            assert!(
                outs.contains(&pkg.join("dist/tsbuildinfo.json").as_path()),
                "{name}: {outs:?}"
            );
            assert!(
                outs.iter()
                    .all(|o| !o.to_string_lossy().contains("configDir")),
                "{name}: {outs:?}"
            );
            let m = out
                .memberships
                .iter()
                .find(|m| m.dir == pkg)
                .unwrap_or_else(|| panic!("{name}: {:?}", out.memberships));
            // Sound: the sources are still inputs.
            assert_eq!(m.pattern, "**/*", "{name}");
            assert!(
                m.exclude.contains(&"node_modules/**".to_string()),
                "{name}: {m:?}"
            );
            assert!(m.exclude.contains(&"dist/**".to_string()), "{name}: {m:?}");
            assert!(
                m.exclude
                    .iter()
                    .all(|e| !e.contains("configDir") && !e.starts_with("src")),
                "{name}: {m:?}"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// `tsc -b` over a solution tsconfig with more than 64 references: every
    /// project counts. (The walk used to stop silently after 64 projects,
    /// dropping `apps/web` from a large monorepo's root `tsc -b`.)
    #[test]
    fn many_project_references_are_all_read() {
        let n = 70;
        let refs: Vec<String> = (0..n)
            .map(|i| format!(r#"{{ "path": "packages/p{i}" }}"#))
            .collect();
        let root_cfg = format!(r#"{{ "files": [], "references": [{}] }}"#, refs.join(", "));
        let mut files: Vec<(String, String)> = vec![("tsconfig.json".into(), root_cfg)];
        for i in 0..n {
            files.push((
                format!("packages/p{i}/tsconfig.json"),
                r#"{ "compilerOptions": { "composite": true, "outDir": "dist" }, "include": ["src"] }"#.into(),
            ));
        }
        let borrowed: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let root = tree("many", &borrowed);
        let mut out = TaskSpec::default();
        add_ts_project(&root.join("tsconfig.json"), true, &mut out, &mut Vec::new());
        for i in 0..n {
            let d = root.join(format!("packages/p{i}/src"));
            assert!(
                out.memberships.iter().any(|m| m.dir == d),
                "p{i} missing ({} memberships)",
                out.memberships.len()
            );
        }
        assert!(out.uncertainties.is_empty(), "{:?}", out.uncertainties);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expands_only_a_leading_config_dir() {
        let leaf = Path::new("/r/pkg");
        assert_eq!(expand_config_dir("${configDir}/dist", leaf), "/r/pkg/dist");
        assert_eq!(expand_config_dir("${configDir}", leaf), "/r/pkg");
        assert_eq!(expand_config_dir("dist", leaf), "dist");
        assert_eq!(expand_config_dir("a/${configDir}", leaf), "a/${configDir}");
    }
}
