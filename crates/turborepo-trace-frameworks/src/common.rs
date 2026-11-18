//! Pieces shared by the adapters: reading config fields with uncertainty
//! records, a minimal module resolver, manual-mock and snapshot lookup.

use std::path::{Path, PathBuf};

use turborepo_trace_core::{UncertaintyKind, eval::Value};
use turborepo_trace_lang_js::extract::config::ConfigVal;

use crate::{
    fsx,
    fsx::PathFsx,
    inputs::{
        ConfigValues, EdgeKind, FrameworkUncertainty, GlobalInputs, ImplicitEdge, InputRef,
        InputRole, MembershipSpec, MockPolicy, MockSite, Project, ResolutionOverrides,
        SnapshotPolicy, Target, TestInputs,
    },
    scan::{CallKind, Scanned},
};

/// Resolves module requests for mock lookup. The tracer's language resolver
/// implements this in the integration; [`SimpleResolver`] covers relative and
/// absolute paths.
pub trait ModuleResolver {
    /// The file `request` resolves to from `from_dir`, if any. Aliases have
    /// already been applied by the caller.
    fn resolve(&self, request: &str, from_dir: &Path) -> Option<PathBuf>;

    /// Every file the resolution of `request` stands for: the module, and with
    /// the tracer's resolver also the manifest that answers for a package
    /// that is not installed, or several files when the request has
    /// alternatives. Default: [`ModuleResolver::resolve`].
    fn resolve_all(&self, request: &str, from_dir: &Path) -> Vec<PathBuf> {
        self.resolve(request, from_dir).into_iter().collect()
    }
}

/// Relative and absolute requests: exact file, then `.<ext>`, then
/// `/index.<ext>`. Bare requests do not resolve.
#[derive(Clone, Debug)]
pub struct SimpleResolver {
    /// Extensions tried, without dot.
    pub extensions: Vec<String>,
}

impl Default for SimpleResolver {
    fn default() -> Self {
        SimpleResolver {
            extensions: ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "json"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl ModuleResolver for SimpleResolver {
    fn resolve(&self, request: &str, from_dir: &Path) -> Option<PathBuf> {
        if !is_path_request(request) {
            return None;
        }
        let base = fsx::join(from_dir, request);
        if base.fx_is_file() {
            return Some(base);
        }
        let s = base.to_string_lossy();
        for e in &self.extensions {
            let p = PathBuf::from(format!("{s}.{e}"));
            if p.fx_is_file() {
                return Some(p);
            }
        }
        for e in &self.extensions {
            let p = base.join(format!("index.{e}"));
            if p.fx_is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// Whether `r` is a relative or absolute path request (not a package).
pub fn is_path_request(r: &str) -> bool {
    r.starts_with("./") || r.starts_with("../") || r.starts_with('/') || r == "." || r == ".."
}

/// A string-ish config item.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A known string.
    Str(String),
    /// `require.resolve(spec)` from a directory.
    Resolve {
        /// Specifier.
        spec: String,
        /// Directory.
        from_dir: String,
    },
    /// A regex literal.
    Regex {
        /// Source.
        source: String,
        /// Flags.
        flags: String,
    },
}

/// Reads fields of an evaluated config and records what it cannot read.
pub struct Reader<'a> {
    /// The config object.
    pub cfg: ConfigVal,
    /// The file uncertainties point at.
    pub file: PathBuf,
    /// Its source (for field lines).
    pub src: &'a [u8],
    /// Collected uncertainties.
    pub unc: Vec<FrameworkUncertainty>,
}

impl<'a> Reader<'a> {
    /// A reader over `cfg`.
    pub fn new(cfg: ConfigVal, file: PathBuf, src: &'a [u8]) -> Self {
        Reader {
            cfg,
            file,
            src,
            unc: Vec::new(),
        }
    }

    /// The value at the dotted `key` (`test.include`).
    pub fn get(&self, key: &str) -> Option<ConfigVal> {
        let mut cur = self.cfg.clone();
        for part in key.split('.') {
            cur = cur.field(part)?;
        }
        Some(cur)
    }

    /// Records an uncertainty for `field`.
    pub fn report(&mut self, field: &str, kind: UncertaintyKind, detail: String) {
        // One record per construct: an unknown spread makes every field unknown for the
        // same reason; list the fields on one record.
        if let Some(u) = self
            .unc
            .iter_mut()
            .find(|u| u.kind == kind && u.detail == detail)
        {
            if let Some(f) = &mut u.field
                && !f.split(", ").any(|x| x == field)
            {
                f.push_str(", ");
                f.push_str(field);
            }
            return;
        }
        let last = field.rsplit('.').next().unwrap_or(field);
        let line = fsx::line_of_field(self.src, last);
        let u = FrameworkUncertainty {
            kind,
            file: self.file.clone(),
            line,
            field: Some(field.to_string()),
            detail,
        };
        if !self.unc.contains(&u) {
            self.unc.push(u);
        }
    }

    /// Records every unknown part of `v` for `field`.
    pub fn report_unknown(&mut self, field: &str, v: &ConfigVal) {
        let mut rs = Vec::new();
        v.unknown_reasons(&mut rs);
        for r in rs {
            let kind = if r.contains("process.env") {
                UncertaintyKind::EnvDependent
            } else {
                UncertaintyKind::ComputedSpecifier
            };
            self.report(field, kind, r);
        }
    }

    /// String items of `field` (a string or an array, flattened over
    /// alternatives).
    pub fn items(&mut self, field: &str) -> Option<Vec<Item>> {
        let v = self.get(field)?;
        Some(self.items_of(field, &v))
    }

    /// String items of a value.
    pub fn items_of(&mut self, field: &str, v: &ConfigVal) -> Vec<Item> {
        let mut out = Vec::new();
        self.collect(field, v, &mut out);
        out
    }

    fn collect(&mut self, field: &str, v: &ConfigVal, out: &mut Vec<Item>) {
        match v {
            ConfigVal::Str(Value::Str(s)) => out.push(Item::Str(s.clone())),
            ConfigVal::Str(other) => {
                self.report(
                    field,
                    UncertaintyKind::PartialSpecifier,
                    format!("partially known: {}", other.to_specifier()),
                );
            }
            ConfigVal::CwdPath(Value::Str(s)) => {
                self.report(
                    field,
                    UncertaintyKind::CwdDependent,
                    format!("path relative to the working directory: {s}"),
                );
                out.push(Item::Str(format!("<cwd>/{s}")));
            }
            ConfigVal::Resolve { spec, from_dir } => out.push(Item::Resolve {
                spec: spec.clone(),
                from_dir: from_dir.clone(),
            }),
            ConfigVal::Regex { source, flags } => out.push(Item::Regex {
                source: source.clone(),
                flags: flags.clone(),
            }),
            ConfigVal::Arr(xs) => xs.iter().for_each(|x| self.collect(field, x, out)),
            ConfigVal::Alts(xs) => xs.iter().for_each(|x| self.collect(field, x, out)),
            ConfigVal::Null | ConfigVal::Bool(false) => {}
            other => self.report_unknown(field, other),
        }
    }

    /// A single string field.
    pub fn string(&mut self, field: &str) -> Option<String> {
        let items = self.items(field)?;
        let strs: Vec<String> = items
            .into_iter()
            .filter_map(|i| match i {
                Item::Str(s) => Some(s),
                _ => None,
            })
            .collect();
        if strs.len() > 1 {
            self.report(
                field,
                UncertaintyKind::ConditionalResolution,
                format!("one of {} values; using the first", strs.len()),
            );
        }
        strs.into_iter().next()
    }

    /// A boolean field (`None` if missing or not a known boolean).
    pub fn bool(&self, field: &str) -> Option<bool> {
        match self.get(field)? {
            ConfigVal::Bool(b) => Some(b),
            _ => None,
        }
    }
}

/// The [`ConfigValues`] of a code config: the absolute paths that are values of
/// the `modelled` fields (dotted names) of `cfg` and of nothing else, plus the
/// byte ranges of function values the adapter models (`modelled_spans`), plus
/// the reporter references: the values of the properties named in
/// `reporter_fields` ([`ConfigValues::runner`]). `None` unless `file` is the
/// code file `loaded` was evaluated from.
///
/// A path that is also the value of another field, or that reached code the
/// evaluator could not follow ([`crate::jsconfig::Loaded::escaped`]), is left
/// out: something the adapter does not model may read it.
pub fn config_values(
    cfg: &ConfigVal,
    modelled: &[&str],
    loaded: &crate::jsconfig::Loaded,
    file: &Path,
    modelled_spans: Vec<(u32, u32)>,
    reporter_fields: &[&str],
) -> Option<ConfigValues> {
    if loaded.code_files.first().map(PathBuf::as_path) != Some(file) {
        return None;
    }
    fn paths(v: &ConfigVal, depth: u32, out: &mut Vec<PathBuf>) {
        if depth > 16 {
            return;
        }
        match v {
            ConfigVal::Str(Value::Str(s)) if s.starts_with('/') => {
                out.push(fsx::normalize(Path::new(s)));
                // A mapper or alias replacement with group references (`<dir>/$1`, written
                // `${path.resolve(__dirname, 'services')}/$1`): the directory before the
                // first reference is the path the config names.
                if let Some(i) = s.find('$')
                    && let Some(j) = s[..i].rfind('/')
                    && j > 0
                {
                    out.push(fsx::normalize(Path::new(&s[..j])));
                }
            }
            ConfigVal::Resolve { from_dir, .. } => out.push(fsx::normalize(Path::new(from_dir))),
            ConfigVal::Arr(xs) | ConfigVal::Alts(xs) => {
                xs.iter().for_each(|x| paths(x, depth + 1, out))
            }
            ConfigVal::Obj(o) => o.entries.iter().for_each(|(_, x)| paths(x, depth + 1, out)),
            _ => {}
        }
    }
    fn without(v: &ConfigVal, field: &[&str]) -> ConfigVal {
        match (v, field) {
            (ConfigVal::Obj(o), [name]) => {
                let mut o = o.clone();
                o.remove(name);
                ConfigVal::Obj(o)
            }
            (ConfigVal::Obj(o), [name, rest @ ..]) => {
                let mut o = o.clone();
                if let Some(x) = o.get(name).cloned() {
                    o.set(*name, without(&x, rest));
                }
                ConfigVal::Obj(o)
            }
            (ConfigVal::Alts(xs), _) => {
                ConfigVal::Alts(xs.iter().map(|x| without(x, field)).collect())
            }
            _ => v.clone(),
        }
    }
    let mut consumed = Vec::new();
    // Everything else the file exports: for an inline project this includes the
    // project objects themselves (`projects` is not a modelled field of the
    // file), so nothing qualifies there.
    let mut rest = loaded.value.clone().unwrap_or(ConfigVal::Null);
    for field in modelled {
        let parts: Vec<&str> = field.split('.').collect();
        let mut cur = Some(cfg.clone());
        for p in &parts {
            cur = cur.and_then(|c| c.field(p));
        }
        if let Some(v) = cur {
            paths(&v, 0, &mut consumed);
        }
        rest = without(&rest, &parts);
    }
    let mut other = Vec::new();
    paths(&rest, 0, &mut other);
    other.extend(
        loaded
            .escaped
            .iter()
            .filter(|s| s.starts_with('/'))
            .map(|s| fsx::normalize(Path::new(s))),
    );
    consumed.retain(|p| !other.contains(p));
    consumed.sort();
    consumed.dedup();
    // Reporter references: the values of the reporter fields, and the imports whose
    // bindings are used only there.
    let mut runner: Vec<(u32, u32)> = loaded
        .properties
        .iter()
        .filter(|(name, _)| reporter_fields.contains(&name.as_str()))
        .map(|(_, span)| *span)
        .collect();
    let reporter_imports: Vec<&turborepo_trace_lang_js::extract::config::ImportInfo> = loaded
        .imports
        .iter()
        .filter(|i| {
            !i.uses.is_empty()
                && i.uses
                    .iter()
                    .all(|&u| runner.iter().any(|&(a, b)| a <= u && u < b))
        })
        .collect();
    // Helper modules the evaluator loaded only for such an import (the specifier is
    // written once in the file), with what they loaded in turn, unless another
    // load reaches them.
    let quoted = |spec: &str| {
        ['\'', '"', '`']
            .iter()
            .map(|q| {
                let needle = format!("{q}{spec}{q}");
                loaded
                    .src
                    .windows(needle.len())
                    .filter(|w| *w == needle.as_bytes())
                    .count()
            })
            .sum::<usize>()
    };
    let only_reporter =
        |spec: &str| reporter_imports.iter().any(|i| i.spec == spec) && quoted(spec) == 1;
    let mut runner_files: Vec<PathBuf> = loaded
        .helper_loads
        .iter()
        .filter(|(spec, _)| only_reporter(spec))
        .flat_map(|(_, files)| files.iter().cloned())
        .collect();
    for (spec, files) in &loaded.helper_loads {
        if !only_reporter(spec) {
            runner_files.retain(|f| !files.contains(f));
        }
    }
    runner_files.retain(|f| f != file);
    runner.extend(reporter_imports.iter().map(|i| i.span));
    Some(ConfigValues {
        file: file.to_path_buf(),
        paths: consumed,
        modelled: modelled_spans,
        opaque: loaded.opaque.clone(),
        runner,
        runner_files,
    })
}

/// Takes the helper files that only a reporter import loaded
/// ([`ConfigValues::runner_files`]) out of a project's config inputs, with the
/// evaluation notes about them: no test executes them. They stay reachable from
/// the config module's own import, as data.
pub fn drop_runner_files(global: &mut GlobalInputs, unc: &mut Vec<FrameworkUncertainty>) {
    let Some(v) = &global.values else {
        return;
    };
    if v.runner_files.is_empty() {
        return;
    }
    let files = v.runner_files.clone();
    global.config_files.retain(|f| !files.contains(f));
    global.js_entries.retain(|f| !files.contains(f));
    unc.retain(|u| !files.contains(&u.file));
}

/// Manual mock lookup and automock entries for one module request in a test.
pub struct MockCx<'a> {
    /// The project.
    pub project: &'a Project,
    /// Resolver.
    pub resolver: &'a dyn ModuleResolver,
    /// The test file's directory.
    pub test_dir: &'a Path,
}

impl MockCx<'_> {
    /// Applies aliases, then resolves `request` from the test directory.
    pub fn resolve(&self, request: &str) -> Option<PathBuf> {
        resolve_with_aliases(
            &self.project.resolution,
            self.resolver,
            request,
            self.test_dir,
        )
    }

    /// Adds the inputs for a mocked module (no factory): an edge to the manual
    /// mock if it exists, probes for where one could be added, and an
    /// automock entry if none exists.
    pub fn mocked(&self, request: &str, why: &str, out: &mut TestInputs) {
        let pol: &MockPolicy = &self.project.mocks;
        let mut found = false;
        // jest haste mocks: `__mocks__/<request>` anywhere in the roots.
        if let Some((_, p)) = pol.haste_mocks.iter().find(|(k, _)| k == request) {
            push_edge(out, EdgeKind::ManualMock, p.clone(), why);
            found = true;
        }
        let resolved = self.resolve(request);
        let is_pkg =
            !is_path_request(request) && resolved.as_ref().is_none_or(|p| fsx::in_node_modules(p));
        if is_pkg {
            for d in &pol.root_mock_dirs {
                let base = d.join("__mocks__").join(request);
                for e in &pol.extensions {
                    let p = PathBuf::from(format!("{}.{e}", base.to_string_lossy()));
                    if p.fx_is_file() {
                        push_edge(out, EdgeKind::ManualMock, p, why);
                        found = true;
                    } else {
                        push_probe(out, p);
                    }
                }
            }
        } else if let Some(f) = &resolved {
            // Adjacent `__mocks__/<basename>`.
            if let (Some(dir), Some(name)) = (f.parent(), f.file_name()) {
                let p = dir.join("__mocks__").join(name);
                if p.fx_is_file() {
                    push_edge(out, EdgeKind::ManualMock, p, why);
                    found = true;
                } else {
                    push_probe(out, p);
                }
            }
        } else {
            // Unresolved path request: every extension is a candidate.
            let base = fsx::join(self.test_dir, request);
            if let (Some(dir), Some(name)) = (base.parent(), base.file_name()) {
                let name = name.to_string_lossy();
                let has_ext = Path::new(name.as_ref()).extension().is_some();
                let mut cands = vec![];
                if has_ext {
                    cands.push(dir.join("__mocks__").join(name.as_ref()));
                }
                for e in &pol.extensions {
                    cands.push(dir.join("__mocks__").join(format!("{name}.{e}")));
                }
                for p in cands {
                    if p.fx_is_file() {
                        push_edge(out, EdgeKind::ManualMock, p, why);
                        found = true;
                    } else {
                        push_probe(out, p);
                    }
                }
            }
        }
        if !found {
            push_entry(
                out,
                InputRole::Automocked,
                Target::Module {
                    request: request.to_string(),
                    from_dir: self.test_dir.to_path_buf(),
                },
            );
        }
    }
}

/// Applies the first matching alias (trying its replacements in order), else
/// resolves `request` directly.
pub fn resolve_with_aliases(
    res: &ResolutionOverrides,
    resolver: &dyn ModuleResolver,
    request: &str,
    from: &Path,
) -> Option<PathBuf> {
    if let Some(hit) = res.apply(request) {
        for r in &hit.requests {
            if let Some(p) = resolver.resolve(r, from) {
                return Some(p);
            }
        }
        return None;
    }
    resolver.resolve(request, from)
}

fn push_edge(out: &mut TestInputs, kind: EdgeKind, to: PathBuf, why: &str) {
    let e = ImplicitEdge {
        kind,
        to: fsx::normalize(&to),
        reason: why.to_string(),
    };
    if !out.edges.contains(&e) {
        out.edges.push(e);
    }
}

fn push_probe(out: &mut TestInputs, p: PathBuf) {
    let p = fsx::normalize(&p);
    if !out.probes.contains(&p) {
        out.probes.push(p);
    }
}

/// Adds an entry (deduplicated).
pub fn push_entry(out: &mut TestInputs, role: InputRole, target: Target) {
    let r = InputRef { role, target };
    if !out.entries.contains(&r) {
        out.entries.push(r);
    }
}

/// The per-test inputs common to jest and vitest: snapshot, mock calls,
/// actual-module loads. `env_entry` maps an environment pragma to a module
/// request.
pub fn common_test_inputs(
    project: &Project,
    test: &Path,
    scanned: &Scanned,
    resolver: &dyn ModuleResolver,
    env_entry: &dyn Fn(&str) -> Option<String>,
) -> TestInputs {
    let mut out = TestInputs {
        test: test.to_path_buf(),
        ..TestInputs::default()
    };
    let dir = test.parent().unwrap_or(Path::new("/"));
    let name = test
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Snapshots.
    match &project.snapshots {
        SnapshotPolicy::Default => {
            let snap_dir = dir.join("__snapshots__");
            let snap = snap_dir.join(format!("{name}.snap"));
            out.memberships.push(MembershipSpec {
                dir: snap_dir,
                pattern: format!("{name}.snap"),
                reason: "default snapshot path".into(),
            });
            if snap.fx_is_file() {
                push_edge(&mut out, EdgeKind::Snapshot, snap, "default snapshot path");
            }
        }
        SnapshotPolicy::Custom { .. } => {
            out.memberships.push(MembershipSpec {
                dir: project.root_dir.clone(),
                pattern: "**/*.snap".into(),
                reason: "custom snapshot resolver".into(),
            });
            out.uncertainties.push(FrameworkUncertainty {
                kind: UncertaintyKind::UnsupportedSyntax,
                file: test.to_path_buf(),
                line: 0,
                field: None,
                detail: "custom snapshot resolver: snapshot path not computed; any .snap file \
                         under the root counts"
                    .into(),
            });
        }
    }
    // Environment pragma.
    if let Some(env) = &scanned.environment
        && let Some(req) = env_entry(env)
    {
        push_entry(
            &mut out,
            InputRole::TestEnvironment,
            Target::Module {
                request: req,
                from_dir: project.root_dir.clone(),
            },
        );
    }
    let cx = MockCx {
        project,
        resolver,
        test_dir: dir,
    };
    for c in &scanned.calls {
        let Some(req) = &c.request else {
            if c.kind == CallKind::EnableAutomock {
                out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::UnsupportedSyntax,
                    file: test.to_path_buf(),
                    line: c.line,
                    field: None,
                    detail: format!(
                        "{}(): every imported module is automocked (adjacent __mocks__ apply)",
                        c.call
                    ),
                });
            } else {
                out.uncertainties.push(FrameworkUncertainty {
                    kind: UncertaintyKind::ComputedSpecifier,
                    file: test.to_path_buf(),
                    line: c.line,
                    field: None,
                    detail: format!("{}(...) with a computed module", c.call),
                });
            }
            continue;
        };
        out.mocks.push(MockSite {
            call: c.call.clone(),
            request: req.clone(),
            factory: c.factory,
            line: c.line,
        });
        let why = format!("{}('{req}')", c.call);
        match c.kind {
            CallKind::Mock if !c.factory => cx.mocked(req, &why, &mut out),
            CallKind::RequireMock => cx.mocked(req, &why, &mut out),
            CallKind::RequireActual => push_entry(
                &mut out,
                InputRole::RequireActual,
                Target::Module {
                    request: req.clone(),
                    from_dir: dir.to_path_buf(),
                },
            ),
            _ => {}
        }
    }
    out
}

/// A module item as an input target: `<rootDir>` substituted, relative requests
/// made absolute against `base` when they name an existing file.
pub fn module_target(item: &Item, base: &Path, root_dir: &Path) -> Option<Target> {
    let (req, from) = match item {
        Item::Str(s) => (replace_root_dir(s, root_dir), base.to_path_buf()),
        Item::Resolve { spec, from_dir } => (spec.clone(), PathBuf::from(from_dir)),
        Item::Regex { .. } => return None,
    };
    let req = match req.strip_prefix("<cwd>/") {
        Some(r) => fsx::join(root_dir, r).to_string_lossy().into_owned(),
        None => req,
    };
    if is_path_request(&req) {
        let p = fsx::join(&from, &req);
        if p.fx_is_file() {
            return Some(Target::Path(p));
        }
        if let Some(r) = crate::jsconfig::resolve_local(&p) {
            return Some(Target::Path(r));
        }
        return Some(Target::Module {
            request: p.to_string_lossy().into_owned(),
            from_dir: from,
        });
    }
    Some(Target::Module {
        request: req,
        from_dir: from,
    })
}

/// jest `replaceRootDirInPath`: a leading `<rootDir>` becomes the root
/// directory.
pub fn replace_root_dir(s: &str, root: &Path) -> String {
    match s.strip_prefix("<rootDir>") {
        Some(rest) => fsx::join(root, &format!("./{rest}"))
            .to_string_lossy()
            .into_owned(),
        None => s.to_string(),
    }
}
