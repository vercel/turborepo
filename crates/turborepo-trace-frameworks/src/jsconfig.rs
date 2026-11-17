//! Loading config files (JS, TS, JSON, a `package.json` key) through the
//! lang-js config evaluator ([`turborepo_trace_lang_js::extract::config`]).
//!
//! Local helpers (`require('../../jest.config.base')`, `import shared from
//! './shared'`) are followed up to [`LoadOptions::helper_depth`] levels (one by
//! default); deeper ones become unknown values with an uncertainty. JSON is
//! evaluated by the same evaluator (as `module.exports = <json>`), which keeps
//! key order, so `moduleNameMapper` order survives.
//!
//! With [`LoadOptions::follow_factories`] (Playwright), a config that comes
//! from a factory is evaluated too: a workspace package is a helper
//! ([`workspace_module`]), and a helper's exported function is called with the
//! caller's arguments.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use turborepo_trace_core::UncertaintyKind;
use turborepo_trace_lang_js::{
    Dialect,
    extract::config::{
        ConfigHost, ConfigModule, ConfigNote, ConfigVal, FuncVal, FunctionInfo, ImportInfo,
        NoteKind, eval_config, eval_export_call,
    },
};

use crate::{fsx, fsx::PathFsx, inputs::FrameworkUncertainty};

/// Extensions tried for local helper requests.
const HELPER_EXTS: &[&str] = &["js", "cjs", "mjs", "ts", "cts", "mts", "json"];

/// Options for [`load`].
#[derive(Clone, Copy)]
pub struct LoadOptions<'a> {
    /// How many levels of local helpers to follow.
    pub helper_depth: u32,
    /// For `package.json`: the key holding the config (`"jest"`).
    pub json_key: Option<&'a str>,
    /// Known values of package members (`vitest/config` `configDefaults`).
    pub externals: &'a dyn Fn(&str, &[String]) -> Option<ConfigVal>,
    /// Whether config factories are followed (`defineConfig(__dirname)` of a
    /// shared module): a bare specifier that names a first-party workspace
    /// package ([`workspace_module`]) is loaded like a local helper, and a
    /// function that a helper exports is called with the
    /// caller's arguments. What such factories choose by the environment is
    /// kept as alternatives: a lookup `table[key]` with a key that is not
    /// statically known is any of the table's values. Off by default: a
    /// package import is a package reference whose `defineConfig(x)` is
    /// `x`, and a call of a helper's function and such a lookup are
    /// unknown.
    pub follow_factories: bool,
}

fn no_externals(_: &str, _: &[String]) -> Option<ConfigVal> {
    None
}

impl Default for LoadOptions<'_> {
    fn default() -> Self {
        LoadOptions {
            helper_depth: 1,
            json_key: None,
            externals: &no_externals,
            follow_factories: false,
        }
    }
}

/// A loaded config file.
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    /// The config object (`None` if the file exports nothing or could not be
    /// read).
    pub value: Option<ConfigVal>,
    /// The file and every helper read (in order).
    pub files: Vec<PathBuf>,
    /// The code files among them (JSON excluded).
    pub code_files: Vec<PathBuf>,
    /// Evaluation problems (statement-level), attributed to the file they occur
    /// in.
    pub uncertainties: Vec<FrameworkUncertainty>,
    /// Informational notes (conditional assignments, function configs).
    pub notes: Vec<(PathBuf, ConfigNote)>,
    /// The config file's source.
    pub src: Vec<u8>,
    /// Byte ranges of the config file the evaluator could not follow
    /// ([`ConfigModule::opaque`]).
    pub opaque: Vec<(u32, u32)>,
    /// Known strings that reached code the evaluator could not follow, in the
    /// config or a helper ([`ConfigModule::escaped`]).
    pub escaped: Vec<String>,
    /// The functions defined in the config file.
    pub functions: Vec<FunctionInfo>,
    /// The function the file exports, if it exports one
    /// ([`ConfigModule::exported_function`]).
    pub exported_function: Option<FunctionInfo>,
    /// Where property values are written in the config file
    /// ([`ConfigModule::properties`]).
    pub properties: Vec<(String, (u32, u32))>,
    /// The import declarations of the config file ([`ConfigModule::imports`]).
    pub imports: Vec<ImportInfo>,
    /// The local modules the config file itself loads (and the workspace
    /// packages, with [`LoadOptions::follow_factories`]), by specifier,
    /// each with the files loading it added to [`Loaded::files`] (the
    /// module and what it loads in turn).
    pub helper_loads: Vec<(String, Vec<PathBuf>)>,
}

impl Loaded {
    /// The definition of the function value `f`, if the config file itself
    /// defines it.
    pub fn function(&self, f: &FuncVal) -> Option<&FunctionInfo> {
        self.functions.iter().find(|x| x.defines(f))
    }
}

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    code_files: Vec<PathBuf>,
    notes: Vec<(PathBuf, ConfigNote)>,
    escaped: Vec<String>,
    /// The helper depth of the config file itself.
    top_depth: u32,
    /// What each local module the config file loads added to `files`.
    helper_loads: Vec<(String, Vec<PathBuf>)>,
    /// Calls of helper functions made so far ([`MAX_FACTORY_CALLS`]).
    factory_calls: u32,
    /// The repository root ([`repo_root`]) of the config, once asked for.
    root: Option<Option<PathBuf>>,
}

/// What is the same for every module of one [`load`].
#[derive(Clone, Copy)]
struct Cx<'a> {
    state: &'a RefCell<State>,
    externals: &'a dyn Fn(&str, &[String]) -> Option<ConfigVal>,
    follow_factories: bool,
}

struct FsHost<'a> {
    dir: PathBuf,
    depth_left: u32,
    cx: Cx<'a>,
}

impl ConfigHost for FsHost<'_> {
    fn load(&mut self, spec: &str) -> Result<ConfigModule, String> {
        let file = resolve_local(&real(&fsx::join(&self.dir, spec)));
        self.load_recorded(spec, file.ok_or_else(|| "not found".to_string()))
    }

    fn external(&mut self, module: &str, path: &[String]) -> Option<ConfigVal> {
        (self.cx.externals)(module, path)
    }

    fn realpath(&mut self, path: &str) -> Option<String> {
        fsx::realpath(&real(Path::new(path))).map(|p| seen(&p).to_string_lossy().into_owned())
    }

    fn load_package(&mut self, spec: &str) -> Option<Result<ConfigModule, String>> {
        if !self.cx.follow_factories {
            return None;
        }
        let file = self.workspace_module(spec)?;
        Some(self.load_recorded(spec, file))
    }

    fn factories(&mut self) -> bool {
        self.cx.follow_factories
    }

    fn call_export(&mut self, spec: &str, export: &str, args: &[ConfigVal]) -> Option<ConfigVal> {
        if !self.cx.follow_factories || self.depth_left == 0 {
            return None;
        }
        let file = if crate::common::is_path_request(spec) {
            resolve_local(&real(&fsx::join(&self.dir, spec)))?
        } else {
            self.workspace_module(spec)?.ok()?
        };
        {
            let mut st = self.cx.state.borrow_mut();
            if st.factory_calls >= MAX_FACTORY_CALLS {
                return None;
            }
            st.factory_calls += 1;
        }
        let src = fsx::read(&file).ok()?;
        // The module is evaluated again, with the call: its values are not kept between
        // evaluations, and its notes are the same ones.
        let mut host = FsHost {
            dir: file.parent().unwrap_or(Path::new("/")).to_path_buf(),
            depth_left: self.depth_left - 1,
            cx: self.cx,
        };
        let (m, v) = eval_export_call(
            &src,
            Dialect::from_path(&file),
            &seen(&file).to_string_lossy(),
            &mut host,
            export,
            args.to_vec(),
        );
        let mut st = self.cx.state.borrow_mut();
        for n in m.notes {
            let n = (file.clone(), n);
            if !st.notes.contains(&n) {
                st.notes.push(n);
            }
        }
        st.escaped.extend(m.escaped);
        Some(v)
    }
}

impl FsHost<'_> {
    /// Loads the helper `file` that `spec` names, recording what the config
    /// file itself loads ([`Loaded::helper_loads`]).
    fn load_recorded(
        &mut self,
        spec: &str,
        file: Result<PathBuf, String>,
    ) -> Result<ConfigModule, String> {
        let before = self.cx.state.borrow().files.len();
        let r = file.and_then(|f| self.load_helper(f));
        let mut st = self.cx.state.borrow_mut();
        if self.depth_left == st.top_depth {
            let added = st.files[before..].to_vec();
            st.helper_loads.push((spec.to_string(), added));
        }
        r
    }

    fn load_helper(&mut self, file: PathBuf) -> Result<ConfigModule, String> {
        if self.depth_left == 0 {
            // Still an input: editing it affects the config.
            self.cx.state.borrow_mut().files.push(file);
            return Err("helper depth limit".into());
        }
        let src = fsx::read(&file).map_err(|e| format!("unreadable: {e}"))?;
        Ok(eval_file(&file, &src, None, self.depth_left - 1, self.cx))
    }

    /// [`workspace_module`] of `spec` from this module's directory.
    fn workspace_module(&self, spec: &str) -> Option<Result<PathBuf, String>> {
        let root = {
            let mut st = self.cx.state.borrow_mut();
            st.root
                .get_or_insert_with(|| repo_root(&self.dir))
                .clone()?
        };
        workspace_module(&self.dir, spec, &root)
    }
}

thread_local! {
    /// The repository root and the place it is taken to be at ([`as_if`]).
    static AS_IF: RefCell<Option<(PathBuf, PathBuf)>> = const { RefCell::new(None) };
}

/// Runs `f` with every config module loaded on this thread located **as if the
/// repository at `repo` were at `place`**: `__dirname`, `__filename` and
/// `import.meta.url` of a module name its file below `place`, and a path that
/// the config computes from them and hands back (a module to load, a path to
/// resolve) is taken below `repo` again. The files that are read are the real
/// ones.
///
/// What a config computes from its own location comes out with `place` in its
/// text. When `place` is a directory whose names no config spells
/// ([`crate::location::NEUTRAL_ROOTS`]), every occurrence of it in a value is
/// the config's location and nothing the config wrote: the value says how it
/// depends on where the repository is, which a load at the real place cannot
/// say (there, `path.join(__dirname, 'src')` and a path the config spells out
/// are the same text).
pub fn as_if<R>(repo: &Path, place: &Path, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<(PathBuf, PathBuf)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            AS_IF.with(|a| *a.borrow_mut() = self.0.take());
        }
    }
    let before = AS_IF.with(|a| a.replace(Some((repo.to_path_buf(), place.to_path_buf()))));
    let _restore = Restore(before);
    f()
}

/// `p` moved from below `from` to below `to`; `p` itself when it is not below
/// `from`.
fn moved(p: &Path, from: &Path, to: &Path) -> PathBuf {
    match p.strip_prefix(from) {
        Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
        Ok(rest) => to.join(rest),
        Err(_) => p.to_path_buf(),
    }
}

/// The path of the file `p` as a config module sees it ([`as_if`]).
fn seen(p: &Path) -> PathBuf {
    AS_IF.with(|a| match &*a.borrow() {
        Some((repo, place)) => moved(p, repo, place),
        None => p.to_path_buf(),
    })
}

/// The file that a path a config module hands back names ([`as_if`]).
fn real(p: &Path) -> PathBuf {
    AS_IF.with(|a| match &*a.borrow() {
        Some((repo, place)) => moved(p, place, repo),
        None => p.to_path_buf(),
    })
}

thread_local! {
    /// The configs whose projects are being loaded on this thread ([`listing_projects`]).
    static LISTING: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` as the load of the projects that the config `path` lists, unless
/// that load is under way on this thread already: `None` then. A list of
/// projects that names the config's own directory (`projects: ['<rootDir>',
/// '<rootDir>/examples/*']`), or a glob that matches it, names the config
/// itself: as a project it is its own options without the list, as the runners
/// take it (a load that followed the list again would not end).
pub fn listing_projects<R>(path: &Path, f: impl FnOnce() -> R) -> Option<R> {
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            LISTING.with(|l| {
                l.borrow_mut().pop();
            });
        }
    }
    if LISTING.with(|l| l.borrow().iter().any(|p| p == path)) {
        return None;
    }
    LISTING.with(|l| l.borrow_mut().push(path.to_path_buf()));
    let _done = Done;
    Some(f())
}

/// How many calls of helper functions one [`load`] evaluates (each evaluates
/// the helper module again); further calls are unknown.
const MAX_FACTORY_CALLS: u32 = 64;

/// Directories of build output whose source is conventionally in `src/`.
const OUTPUT_DIRS: &[&str] = &["dist", "build", "lib", "out"];

/// The repository of `dir`: the nearest ancestor with a `.git` entry, else the
/// outermost ancestor with a `package.json`.
fn repo_root(dir: &Path) -> Option<PathBuf> {
    let mut outermost = None;
    for d in dir.ancestors() {
        if d.join(".git").fx_exists() {
            return Some(d.to_path_buf());
        }
        if d.join("package.json").fx_is_file() {
            outermost = Some(d.to_path_buf());
        }
    }
    outermost
}

/// The package name and the subpath (`""` for the package itself) of a bare
/// specifier.
fn split_package_spec(spec: &str) -> Option<(&str, &str)> {
    if spec.is_empty()
        || spec.starts_with(['.', '/', '#'])
        || spec.contains([':', '\\'])
        || spec.split('/').any(|s| s.is_empty() || s == "..")
    {
        return None;
    }
    let name_end = match (spec.starts_with('@'), spec.find('/')) {
        (true, Some(i)) => spec[i + 1..].find('/').map_or(spec.len(), |j| i + 1 + j),
        (true, None) => return None,
        (false, i) => i.unwrap_or(spec.len()),
    };
    Some((&spec[..name_end], spec[name_end..].trim_start_matches('/')))
}

/// The source file of the bare specifier `spec`, imported by a module in
/// `from_dir`, when its package is a first-party workspace package of the
/// repository `root`: the package is found as Node finds it
/// (`node_modules/<name>` in `from_dir` or an ancestor, up to `root`),
/// and its real path is inside `root` and not inside a `node_modules` directory
/// (a link the package manager made to a workspace member). `None` for every
/// other package (installed, or not found): it is not followed. `Some(Err)`
/// when the workspace package has no file for the specifier.
///
/// The file is chosen from `package.json` (`exports` for the subpath, with a
/// `*` pattern; else `main`/`module`/`types` or the plain subpath), preferring
/// one that exists: a runtime target (`import`, `require`, `default`, ...),
/// else a `types` target that is TypeScript source (not a declaration file),
/// else the conventional source of a missing build output (`./dist/a/b.js` is
/// `./src/a/b.ts`): a checkout has no `dist/`.
pub fn workspace_module(
    from_dir: &Path,
    spec: &str,
    root: &Path,
) -> Option<Result<PathBuf, String>> {
    let (name, subpath) = split_package_spec(spec)?;
    let real_root = fsx::realpath(root)?;
    let mut found = None;
    for d in from_dir.ancestors() {
        let cand = d.join("node_modules").join(name);
        if cand.fx_is_dir() {
            found = Some(cand);
            break;
        }
        if d == root {
            break;
        }
    }
    let real = fsx::realpath(&found?)?;
    if fsx::in_node_modules(&real) {
        return None;
    }
    // In terms of `root` as the caller names it (it may itself be reached through a
    // link).
    let pkg = root.join(real.strip_prefix(&real_root).ok()?);
    Some(package_source(&pkg, subpath).ok_or_else(|| {
        format!(
            "no source file in the workspace package {}",
            pkg.strip_prefix(root).unwrap_or(&pkg).display()
        )
    }))
}

/// The file of `subpath` (`""`: the package itself) in the package directory
/// `pkg`; see [`workspace_module`].
fn package_source(pkg: &Path, subpath: &str) -> Option<PathBuf> {
    // The entry points are fields of the manifest's resolution view.
    let json: serde_json::Value = fsx::read_manifest(pkg.join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);
    // Targets as written (`./dist/config/index.js`), and whether a `types`
    // condition names it.
    let mut targets: Vec<(String, bool)> = Vec::new();
    if let Some(exports) = json.get("exports") {
        let key = if subpath.is_empty() {
            ".".to_string()
        } else {
            format!("./{subpath}")
        };
        export_targets(exports, &key, &mut targets);
    }
    if targets.is_empty() {
        if subpath.is_empty() {
            for (field, types) in [
                ("module", false),
                ("main", false),
                ("types", true),
                ("typings", true),
            ] {
                if let Some(s) = json.get(field).and_then(|v| v.as_str()) {
                    targets.push((s.to_string(), types));
                }
            }
            targets.push(("./index".to_string(), false));
        } else {
            targets.push((format!("./{subpath}"), false));
        }
    }
    // Only files of the package.
    targets.retain(|(t, _)| !t.starts_with('/') && !t.split('/').any(|s| s == ".."));
    let is_declaration = |t: &str| [".d.ts", ".d.mts", ".d.cts"].iter().any(|e| t.ends_with(e));
    let is_ts = |t: &str| {
        [".ts", ".tsx", ".mts", ".cts"]
            .iter()
            .any(|e| t.ends_with(e))
    };
    // A runtime target that exists.
    for (t, _) in targets.iter().filter(|(_, types)| !types) {
        if let Some(f) = resolve_local(&fsx::join(pkg, t)) {
            return Some(f);
        }
    }
    // A `types` target that is source.
    for (t, _) in targets.iter().filter(|(_, types)| *types) {
        let f = fsx::join(pkg, t);
        if is_ts(t) && !is_declaration(t) && f.fx_is_file() {
            return Some(f);
        }
    }
    // The source of a build output that is not there.
    for (t, _) in &targets {
        let parts: Vec<&str> = t
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        let Some(out_dir) = parts.iter().position(|s| OUTPUT_DIRS.contains(s)) else {
            continue;
        };
        let Some((file, dirs)) = parts.split_last() else {
            continue;
        };
        let stem = [".d.ts", ".d.mts", ".d.cts", ".js", ".mjs", ".cjs", ".jsx"]
            .iter()
            .find_map(|e| file.strip_suffix(e))
            .unwrap_or(file);
        // `dist/` replaced by `src/`, or dropped.
        for src_dir in [Some("src"), None] {
            let mut dir = pkg.to_path_buf();
            for (i, s) in dirs.iter().enumerate() {
                match (i == out_dir, src_dir) {
                    (true, Some(src)) => dir.push(src),
                    (true, None) => {}
                    (false, _) => dir.push(s),
                }
            }
            for ext in ["ts", "tsx", "mts", "cts"] {
                let f = dir.join(format!("{stem}.{ext}"));
                if f.fx_is_file() {
                    return Some(f);
                }
            }
        }
    }
    None
}

/// The targets `exports` gives for the subpath `key` (`.` or `./x`), from every
/// condition.
fn export_targets(exports: &serde_json::Value, key: &str, out: &mut Vec<(String, bool)>) {
    fn leaves(
        v: &serde_json::Value,
        star: Option<&str>,
        types: bool,
        depth: u32,
        out: &mut Vec<(String, bool)>,
    ) {
        if depth > 8 || out.len() >= 64 {
            return;
        }
        match v {
            serde_json::Value::String(s) => {
                let s = match star {
                    Some(star) => s.replace('*', star),
                    None => s.clone(),
                };
                if !out.iter().any(|(t, _)| *t == s) {
                    out.push((s, types));
                }
            }
            serde_json::Value::Array(xs) => xs
                .iter()
                .for_each(|x| leaves(x, star, types, depth + 1, out)),
            serde_json::Value::Object(o) => o
                .iter()
                .for_each(|(cond, x)| leaves(x, star, types || cond == "types", depth + 1, out)),
            _ => {}
        }
    }
    let subpaths = match exports {
        serde_json::Value::Object(o) if o.keys().any(|k| k.starts_with('.')) => o,
        // A string, an array, or conditions: the package itself.
        v => {
            if key == "." {
                leaves(v, None, false, 0, out);
            }
            return;
        }
    };
    if let Some(v) = subpaths.get(key) {
        leaves(v, None, false, 0, out);
        return;
    }
    for (pattern, v) in subpaths {
        if let Some((pre, post)) = pattern.split_once('*')
            && key.len() >= pre.len() + post.len()
            && key.starts_with(pre)
            && key.ends_with(post)
        {
            leaves(
                v,
                Some(&key[pre.len()..key.len() - post.len()]),
                false,
                0,
                out,
            );
        }
    }
}

/// `base` as a file: exact, with an extension, or `index.<ext>`.
pub fn resolve_local(base: &Path) -> Option<PathBuf> {
    if base.fx_is_file() {
        return Some(base.to_path_buf());
    }
    let s = base.to_string_lossy();
    for e in HELPER_EXTS {
        let p = PathBuf::from(format!("{s}.{e}"));
        if p.fx_is_file() {
            return Some(p);
        }
    }
    for e in HELPER_EXTS {
        let p = base.join(format!("index.{e}"));
        if p.fx_is_file() {
            return Some(p);
        }
    }
    None
}

fn eval_file(
    file: &Path,
    src: &[u8],
    json_key: Option<&str>,
    depth_left: u32,
    cx: Cx<'_>,
) -> ConfigModule {
    let state = cx.state;
    let is_json = file.extension().is_some_and(|e| e == "json");
    {
        let mut st = state.borrow_mut();
        st.files.push(file.to_path_buf());
        if !is_json {
            st.code_files.push(file.to_path_buf());
        }
    }
    let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut host = FsHost {
        dir,
        depth_left,
        cx,
    };
    let path_str = seen(file).to_string_lossy().into_owned();
    let mut m = if is_json {
        let mut wrapped = b"module.exports = ".to_vec();
        wrapped.extend_from_slice(src);
        eval_config(&wrapped, Dialect::JS, &path_str, &mut host)
    } else {
        eval_config(src, Dialect::from_path(file), &path_str, &mut host)
    };
    if let Some(k) = json_key {
        let v = m.cjs.as_ref().and_then(|c| c.field(k));
        m.cjs = v;
    }
    let notes = std::mem::take(&mut m.notes);
    let mut st = state.borrow_mut();
    for n in notes.into_iter().map(|n| (file.to_path_buf(), n)) {
        // A factory is evaluated once per call, and loads its helpers each time.
        if !(cx.follow_factories && st.notes.contains(&n)) {
            st.notes.push(n);
        }
    }
    st.escaped.extend(m.escaped.iter().cloned());
    drop(st);
    m
}

/// Loads the config file `path`.
pub fn load(path: &Path, opts: LoadOptions<'_>) -> Loaded {
    let src = match fsx::read(path) {
        Ok(s) => s,
        Err(e) => {
            return Loaded {
                files: vec![path.to_path_buf()],
                uncertainties: vec![FrameworkUncertainty {
                    kind: UncertaintyKind::ParseIncomplete,
                    file: path.to_path_buf(),
                    line: 0,
                    field: None,
                    detail: format!("unreadable: {e}"),
                }],
                ..Loaded::default()
            };
        }
    };
    let state = RefCell::new(State {
        top_depth: opts.helper_depth,
        ..State::default()
    });
    let m = eval_file(
        path,
        &src,
        opts.json_key,
        opts.helper_depth,
        Cx {
            state: &state,
            externals: opts.externals,
            follow_factories: opts.follow_factories,
        },
    );
    let mut st = state.into_inner();
    st.escaped.sort_unstable();
    st.escaped.dedup();
    let mut out = Loaded {
        value: m.config(),
        files: dedup(st.files),
        code_files: dedup(st.code_files),
        src,
        opaque: m.opaque.clone(),
        escaped: st.escaped,
        functions: m.functions.clone(),
        exported_function: m.exported_function.clone(),
        properties: m.properties.clone(),
        imports: m.imports.clone(),
        helper_loads: st.helper_loads,
        ..Loaded::default()
    };
    for (file, n) in st.notes {
        let kind = match n.kind {
            NoteKind::Unsupported => Some(UncertaintyKind::UnsupportedSyntax),
            NoteKind::NotFollowed => Some(UncertaintyKind::UnresolvedSpecifier),
            NoteKind::EnvDependent => Some(UncertaintyKind::EnvDependent),
            NoteKind::Truncated | NoteKind::ParseIncomplete => {
                Some(UncertaintyKind::ParseIncomplete)
            }
            NoteKind::Conditional | NoteKind::FunctionConfig => None,
        };
        match kind {
            Some(kind) => out.uncertainties.push(FrameworkUncertainty {
                kind,
                file,
                line: n.line,
                field: None,
                detail: n.detail,
            }),
            None => out.notes.push((file, n)),
        }
    }
    out
}

fn dedup(v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}
