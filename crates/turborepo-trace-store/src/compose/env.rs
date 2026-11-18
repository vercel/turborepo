//! The environment registry: paths that travel through environment variables
//! ([`Graph::env_patch`]).
//!
//! `readFileSync(process.env.DATAFILE)` or `readFile(getEnv('TOKEN_PATH',
//! DEFAULT))` name a file through the process environment. The file that reads
//! cannot know the value: it is assigned in other files (a test's setup, the
//! `env` object of a spawned process) or outside the repository. Every
//! first-party file therefore carries facts ([`turborepo_trace_core::env`], a
//! pure function of its text, stored with its extraction in its shard): its
//! *uses* (references whose path is a template over variables and calls of
//! imported helpers), its *assignments* (variable, evaluated value) and its
//! *helpers* (exported functions that return environment values, as templates
//! over their arguments). This module joins them over the composed graph:
//!
//! - the **registry** maps a variable to every value the repository assigns to
//!   it;
//! - every use is evaluated against the registry: variables are replaced by
//!   their assigned values, helper calls by the helper's template with the
//!   call's arguments;
//! - the resulting paths are resolved like ordinary references, on the file
//!   with the use:
//!
//! | Value | Added to the file |
//! |---|---|
//! | a path below the directory of the assigning file (`path.join(__dirname, 'fixtures/a.json')`) that stays in the repository | an edge to the file when it is a node, a config input when it exists without being one, a probe when it is missing; for a directory, its files and a `**` membership; for a path with unknown segments, the files the glob matches and a membership |
//! | an absolute path (`/var/run/secrets/x/token`, `/tmp/flags.json`), a path in the temporary directory, a URL | nothing: outside the repository (an absolute path below the repository root counts as a path in it) |
//! | a relative path (`dev/flags.json`, or `process.cwd()`-relative) | relative to the working directory of whoever runs the reader, which is not known here: every repository file whose path ends with it, and a `**/<path>` membership |
//! | a variable followed by literal text (`${DATA_DIR}/seed.json`) | the assigned values with the text appended; and, because the variable may also be set outside the repository to any directory, every repository file whose path ends with the text |
//! | for a module load: a file-anchored path, or `./x` relative to the place the load searches from | the module candidates (extensions, `index` files) as edges and probes; a bare specifier stays open, and so does a load whose variable the repository never assigns or declares (see below) |
//! | for a module load: a literal path relative to the working directory (`path.resolve(process.cwd(), '../cfg/seed.config.ts')`) | the module candidates in every repository directory that holds one (at most 32 directories), and memberships for a candidate created elsewhere; a path with unknown segments stays open |
//!
//! A module load asked of a `require` function
//! (`createRequire(p)(process.env.X)`) says where that function searches from
//! ([`EnvFrom`]): the file (its own `require`, or a function made for the file
//! or a file next to it), a known directory elsewhere, or a place that is not
//! known. Only a relative value depends on it: it is resolved from that
//! place, and where the place is not known it is not resolved at all (the
//! record stays open and says so). Every other value names the same module for
//! every `require` function.
//!
//! Every file that assigns one of the use's variables becomes a config input of
//! the file with the use: editing an assignment re-runs the readers.
//!
//! When every alternative of a use was evaluated and resolved, its uncertainty
//! record is dropped. It stays open when an assignment of one of its variables
//! cannot be evaluated, when a helper is not a template, or when a value
//! matches too many files.
//!
//! A record is named by its kind, line and span, and a reference has one record
//! per origin of its unknown alternatives. Where another alternative of the
//! reference has a record with the same key (`require(c ? getEnv('X', F) :
//! other)`: the helper's value and the parameter are both `computed_specifier`
//! at the same place), the key is **shared**: the dependencies are added and
//! the records are left as they are, since dropping the key would close the
//! alternative nothing resolved. The layer sees that from the records
//! (one of another origin has the key); for a load asked of its callee, whose
//! records all end with the origin `unknown`, the use says it
//! (`EnvFact::Use::shared`). A variable read directly (`process.env.X ??
//! other`) has a record kind of its own and shares nothing.
//!
//! This layer owns every read that is fed by the environment. A use it cannot
//! resolve keeps a record that says so: `env_dependent` with origin `env` and
//! the reason in its detail, also when the extractor recorded a call of a
//! helper (`computed_specifier`, origin `call`). The name scan
//! (`turborepo_trace_graph::names`) bounds computed reads by the path-like
//! strings of the code loaded with them; a value from the environment is
//! not such a string, so those records must not reach it. A call of a function
//! that is not an environment helper is not this layer's: it stays a computed
//! read, and so does a record that stands for such a call next to a helper's
//! (`read(c ? getEnv('X', F) : pick())`: one record of origin `call` for both),
//! since a file read from the environment is ignored by default and the other
//! function's value is not the environment's ([`env_fed`]).
//!
//! # Values set outside the repository
//!
//! A variable can also be set by something the repository does not contain: a
//! CI setting, a deploy system, a developer's shell. The repository has no file
//! whose change alters such a value, so it adds no dependency *on an
//! assignment*. What the value names is the question:
//!
//! - A path outside the repository (a mounted secret, a temporary file) is no
//!   dependency.
//! - A path inside the repository is a dependency the repository does not
//!   declare. For a template with literal text the registry still covers it:
//!   whatever directory the variable names, the file read ends with the literal
//!   text, and all such repository files are dependencies. For a use that is
//!   the bare variable nothing bounds it, and this module assumes the value is
//!   outside the repository. Declare the exception in `turbotrace.toml`, which
//!   makes the declared paths assigned values of the variable:
//!
//!   ```toml
//!   [[env]]
//!   name = "FLAGS_DATAFILE"
//!   values = ["fixtures/flags.json"] # repository-relative; globs allowed
//!   ```
//!
//! A module load is different from a read: the variable names code. When the
//! repository assigns it (a test, a setup file) or declares it, those values
//! are the loads. When nothing in the repository gives it a value, nothing
//! bounds what runs, and the record stays open; `[[env]]` with the modules (or
//! with `values = []`: nothing in the repository) closes it.
//!
//! A few variables have a meaning by convention: `GITHUB_WORKSPACE` is the
//! repository root; `HOME`, `USERPROFILE`, `PATH`, `APPDATA`, `LOCALAPPDATA`
//! and the `XDG_*` directories are outside the repository unless the repository
//! assigns them.
//!
//! # Soundness
//!
//! A dropped record is sound under these assumptions:
//!
//! 1. *Names are literal where values are given.* The repository gives a
//!    variable a value only in a form the extractor records with the variable's
//!    literal name (see `turborepo_trace_lang_js::flow::env`: member writes,
//!    object keys, name-and-value calls, `NAME=value` in command-line text). A
//!    name computed at runtime (`process.env[prefix + '_PATH'] = v`) or a value
//!    given far from the name (parallel arrays) is not seen.
//! 2. *First-party JS / TS and task command lines.* Assignments are read from
//!    JS / TS files and from the command lines of the graph's tasks (`X=value
//!    cmd` in a package.json script: relative to the package, with the
//!    package.json as the readers' input). Not read: `.env` files, YAML
//!    (workflow `env:`, docker-compose, Kubernetes manifests), shell scripts,
//!    Dockerfiles, other languages. On the two repositories this was sized on,
//!    none of the 174 such assignments of a variable that is read as a path
//!    names a repository path (they are mounted secrets, absolute paths and
//!    plain words). Declare what they set to repository paths with `[[env]]`.
//! 3. *Outside values are outside paths* for a bare variable, as described
//!    above, and for a path below a directory that only something outside the
//!    repository names when the rest of the path is too unspecific to bound
//!    anything (`${DIR}/${id}.json`).
//! 4. The join is global and flow-insensitive: a reader depends on every value
//!    any file assigns to the variable, whichever process runs it. That
//!    over-approximates and never drops a value.
//!
//! # Invalidation
//!
//! Facts live in the shard of their file and are re-extracted exactly when the
//! file's content changes (the shard key). The registry and the resolution are
//! not stored: they are recomputed from the composed graph whenever a graph is
//! opened, like the per-test working directories ([`Graph::cwd_patch`]). An
//! assignment added, changed or removed anywhere therefore changes every
//! reader's edges on the next open, with nothing to invalidate by hand. Paths
//! are checked against the snapshot's walked-file list and the graph's nodes,
//! never against the file system.
//!
//! Recomputing costs tens of milliseconds on a large repository, so the result
//! is kept in the store's cache of derived layers, keyed by the manifest and
//! the declared values ([`Graph::env_patch_cached`]): an unchanged repository
//! reads it back.
//!
//! `TURBO_TRACE_DEBUG_ENV=0` (at build and at query time) turns the mechanism
//! off. `TURBO_TRACE_DEBUG_ENV_REPORT=<file>` writes what every use resolved
//! to, as JSON lines.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use turborepo_trace_core::{
    Phase, RefFlags, RefKind,
    env::{EnvAlt, EnvBase, EnvCall, EnvFact, EnvFrom, EnvName, EnvPart, EnvValue},
};
use turborepo_trace_graph::{
    self as q, EdgeAttr, GraphView, NodeId, Patch,
    glob::{Glob, is_glob},
};

use super::{
    Graph, NONE,
    cwd::{MAX_DIR_FILES, MODULE_EXTS, Resolution, SiteRef, is_module_kind},
};
use crate::shard::kind;

/// Maximum alternatives of one evaluated value.
const MAX_VALUES: usize = 64;
/// Maximum nesting of helper calls and variable references.
const MAX_DEPTH: u32 = 8;
/// Stands for unknown text in an assembled path.
const WILD: char = '\u{0}';

/// A value declared for a variable in `turbotrace.toml` (`[[env]]`): something
/// outside the repository's code sets the variable to these repository paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvDecl {
    /// The variable.
    pub name: String,
    /// Paths or globs relative to the repository root.
    pub values: Vec<String>,
}

/// A cross-file evaluator: asked for every value of the expression at a byte
/// span of a file (an assignment whose value the file's own facts could not
/// evaluate: a parameter bound by callers in other files, the result of an
/// imported function). It answers with alternatives in the facts' own form:
/// text anchored at the repository root ([`EnvBase::Root`]) or the working
/// directory, absolute text, variables, or [`EnvAlt::Unknown`] for what it
/// cannot evaluate either. An empty answer means the expression is never a path
/// in the repository (a temporary file, `undefined`). `None`: it has no answer,
/// and the file's own facts stand. The files it read to answer are
/// reported with [`EnvAnswer::files`]: they become inputs of the variable's
/// readers.
pub type EnvOracle<'a> = dyn Fn(NodeId, [u32; 2]) -> Option<EnvAnswer> + 'a;

/// The answer of an [`EnvOracle`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvAnswer {
    /// The alternatives of the expression's value.
    pub alts: Vec<EnvAlt>,
    /// The files whose content decided the answer (suppliers of arguments,
    /// definitions of called functions), besides the file of the
    /// expression.
    pub files: Vec<NodeId>,
}

/// What [`Graph::env_patch`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvStats {
    /// Uses (references whose path comes from the environment).
    pub uses: usize,
    /// Uses whose uncertainty record was dropped.
    pub resolved: usize,
    /// Uses fed by the environment that stay open: their record says why.
    pub kept: usize,
    /// Variables the uses read.
    pub variables: usize,
    /// Assignment facts of those variables.
    pub assignments: usize,
    /// Helper functions in the graph.
    pub helpers: usize,
    /// Edges added.
    pub edges: usize,
}

/// One assignment of a variable.
struct SetSite {
    node: u32,
    line: u32,
    value: EnvValue,
    /// Byte span of the assigned expression, when the extractor could not
    /// evaluate it.
    at: Option<[u32; 2]>,
}

/// The assignment facts of the graph by variable, decoded on first use: a
/// repository has many assignments (every upper-case object key) and few
/// variables that are read as paths.
#[derive(Default)]
struct SetIndex<'g> {
    /// Variable to `(node, serialized fact)`.
    raw: HashMap<&'g str, Vec<(u32, &'g str)>>,
    decoded: RefCell<HashMap<String, Rc<Vec<SetSite>>>>,
}

impl SetIndex<'_> {
    /// The assignments of `name`.
    fn get(&self, name: &str) -> Option<Rc<Vec<SetSite>>> {
        if let Some(v) = self.decoded.borrow().get(name) {
            return Some(v.clone());
        }
        let raw = self.raw.get(name)?;
        let sites: Vec<SetSite> = raw
            .iter()
            .filter_map(|(node, f)| match EnvFact::decode(f)? {
                EnvFact::Set {
                    line, value, at, ..
                } => Some(SetSite {
                    node: *node,
                    line,
                    value,
                    at,
                }),
                _ => None,
            })
            .collect();
        let sites = Rc::new(sites);
        self.decoded
            .borrow_mut()
            .insert(name.to_owned(), sites.clone());
        Some(sites)
    }

    fn count(&self, name: &str) -> usize {
        self.raw.get(name).map_or(0, Vec::len)
    }
}

/// The walked files by file name, built on first use (for paths relative to an
/// unknown working directory).
#[derive(Default)]
struct NameIndex<'w> {
    by_name: std::cell::OnceCell<HashMap<&'w str, Vec<u32>>>,
}

impl<'w> NameIndex<'w> {
    fn get(&self, walked: &[&'w str]) -> &HashMap<&'w str, Vec<u32>> {
        self.by_name.get_or_init(|| {
            let mut m: HashMap<&'w str, Vec<u32>> = HashMap::new();
            for (i, f) in walked.iter().enumerate() {
                let name = f.rsplit('/').next().unwrap_or(f);
                m.entry(name).or_default().push(i as u32);
            }
            m
        })
    }
}

/// The variable of a serialized assignment fact, without decoding it.
fn set_name(fact: &str) -> Option<&str> {
    let rest = fact.strip_prefix("env:{\"set\":{\"name\":\"")?;
    let name = &rest[..rest.find('"')?];
    (!name.contains('\\')).then_some(name)
}

/// One use.
struct UseSite<'g> {
    node: u32,
    span: [u32; 2],
    line: u32,
    kind: RefKind,
    origin: u8,
    value: &'g EnvValue,
    /// Where a module load of the use searches from.
    from: &'g EnvFrom,
    /// Its record stands for other alternatives too: the records are left
    /// alone.
    shared: bool,
}

/// The facts of the graph: uses and helpers decoded, assignments indexed by
/// variable.
#[derive(Default)]
struct Facts<'g> {
    /// Per fact file: node and its use and helper facts.
    files: Vec<(u32, Vec<EnvFact>)>,
    sets: SetIndex<'g>,
}

/// The anchor of an evaluated value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CBase {
    /// Plain text: absolute, or relative to the reader's working directory.
    Str,
    /// Relative to the working directory.
    Cwd,
    /// Relative to this repository directory.
    Dir(String),
}

/// An evaluated alternative: an anchor and text, where [`WILD`] stands for
/// unknown text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Conc {
    base: CBase,
    text: String,
    /// The text is a variable set outside the repository and nothing else.
    outside: bool,
    /// The text starts with a variable that only something outside the
    /// repository sets (not one whose assignment in the repository could
    /// not be evaluated).
    ext_head: bool,
}

/// Where a term is evaluated: the file its text is in, the calls its parts
/// refer to, and for a helper's template the call being expanded.
#[derive(Clone, Copy)]
struct Cx<'a> {
    node: u32,
    calls: &'a [EnvCall],
    bind: Option<&'a Bind<'a>>,
}

/// The call whose helper template is evaluated, in the caller's context.
struct Bind<'a> {
    call: &'a EnvCall,
    cx: Cx<'a>,
}

/// What a variable means by convention when the repository does not assign it.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Known {
    /// Outside the repository.
    Outside,
    /// The repository root.
    Root,
}

fn known(name: &str) -> Option<Known> {
    match name {
        "GITHUB_WORKSPACE" => Some(Known::Root),
        "HOME" | "USERPROFILE" | "PATH" | "APPDATA" | "LOCALAPPDATA" | "XDG_CACHE_HOME"
        | "XDG_CONFIG_HOME" | "XDG_DATA_HOME" | "XDG_STATE_HOME" | "XDG_RUNTIME_DIR" => {
            Some(Known::Outside)
        }
        _ => None,
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// An assignment in the command line of a task (`X=value cmd` in a package.json
/// script): the processes the command starts see the value.
struct TaskSet {
    /// The task id, for messages.
    task: String,
    /// The node of the file that defines the task (the package's
    /// `package.json`).
    def: Option<u32>,
    /// The directory the command runs in, when it is known (a package script
    /// runs in its package).
    dir: Option<String>,
    value: String,
}

/// The assignments in the command lines of the graph's tasks, by variable.
fn task_sets(g: &Graph) -> HashMap<String, Vec<TaskSet>> {
    let mut out: HashMap<String, Vec<TaskSet>> = HashMap::new();
    for (_, t) in g.tasks() {
        let id = t.id.as_str();
        let package_script = t.sources.iter().any(|s| s.as_str() == "npm");
        // `task:<package dir>#<script>`.
        let dir = id
            .strip_prefix("task:")
            .and_then(|r| r.rsplit_once('#'))
            .map(|(d, _)| d.to_owned())
            .filter(|_| package_script);
        let def = match &dir {
            Some(d) => g.lookup(&crate::paths::join_simple(d, "package.json")),
            None => t
                .runs
                .iter()
                .find_map(|r| r.as_str().strip_prefix("def:").and_then(|p| g.lookup(p))),
        };
        for line in t.lines.iter() {
            for (name, value, _) in turborepo_trace_core::env::command_assignments(line.as_str()) {
                let sets = out.entry(name.to_owned()).or_default();
                if !sets
                    .iter()
                    .any(|s| s.def == def && s.value == value && s.dir == dir)
                {
                    sets.push(TaskSet {
                        task: id.to_owned(),
                        def,
                        dir: dir.clone(),
                        value: value.to_owned(),
                    });
                }
            }
        }
    }
    out
}

/// A literal module path below an unknown directory is resolved in at most this
/// many directories of the repository; more bound nothing.
const MAX_MODULE_DIRS: usize = 32;

/// The evaluator of one use.
struct Eval<'g> {
    g: &'g Graph,
    tasks: &'g HashMap<String, Vec<TaskSet>>,
    oracle: &'g EnvOracle<'g>,
    sets: &'g SetIndex<'g>,
    helpers: &'g HashMap<(u32, &'g str), &'g EnvValue>,
    decls: &'g HashMap<&'g str, Vec<&'g str>>,
    /// Variables being evaluated (a variable assigned from itself adds
    /// nothing).
    visiting: Vec<String>,
    /// Files with an assignment of a variable the use reads.
    sites: Vec<u32>,
    /// Variables the use reads.
    vars: Vec<String>,
    /// One of them is declared in the hints file.
    declared: bool,
}

impl<'g> Eval<'g> {
    fn alts(&mut self, alts: &[EnvAlt], cx: Cx<'_>, depth: u32) -> Result<Vec<Conc>, String> {
        let mut out: Vec<Conc> = Vec::new();
        for a in alts {
            match a {
                EnvAlt::Unknown(r) => return Err(format!("cannot evaluate `{r}`")),
                EnvAlt::Term(t) => {
                    let base = match t.base {
                        EnvBase::Str => CBase::Str,
                        EnvBase::Cwd => CBase::Cwd,
                        EnvBase::Root => CBase::Dir(String::new()),
                        EnvBase::FileDir => {
                            CBase::Dir(dir_of(self.g.path_of(cx.node as usize)).to_owned())
                        }
                    };
                    for c in self.term(base, &t.parts, cx, depth)? {
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                }
            }
            if out.len() > MAX_VALUES {
                return Err("too many alternatives".to_owned());
            }
        }
        Ok(out)
    }

    fn term(
        &mut self,
        base: CBase,
        parts: &[EnvPart],
        cx: Cx<'_>,
        depth: u32,
    ) -> Result<Vec<Conc>, String> {
        if depth > MAX_DEPTH {
            return Err("helper calls nest too deep".to_owned());
        }
        let mut acc = vec![Conc {
            base,
            text: String::new(),
            outside: false,
            ext_head: false,
        }];
        for (i, p) in parts.iter().enumerate() {
            let vals: Vec<Conc> = match p {
                EnvPart::Lit(s) => {
                    for c in &mut acc {
                        c.text.push_str(s);
                        c.outside = false;
                    }
                    continue;
                }
                EnvPart::Wild => {
                    for c in &mut acc {
                        c.text.push(WILD);
                        c.outside = false;
                    }
                    continue;
                }
                EnvPart::Var(name) => {
                    let names: Vec<String> = match name {
                        EnvName::Name(n) => vec![n.clone()],
                        EnvName::Arg(k) => {
                            let mut names = Vec::new();
                            for c in self.arg(cx, *k, false, depth)? {
                                if c.base != CBase::Str || c.text.contains(WILD) {
                                    return Err(
                                        "the variable's name is not a literal string".to_owned()
                                    );
                                }
                                names.push(c.text);
                            }
                            names
                        }
                    };
                    // Next to literal text the variable's value does not have to be
                    // known: whatever directory or name it is, the path contains the
                    // literal text, which bounds it like a value set outside the
                    // repository. The files that assign it are inputs either way.
                    let bounded = parts
                        .iter()
                        .any(|p| matches!(p, EnvPart::Lit(s) if !s.is_empty()));
                    let mut vals = Vec::new();
                    for n in names {
                        match self.variable(&n, depth) {
                            Ok(v) => vals.extend(v),
                            Err(_) if bounded => vals.push(Conc {
                                base: CBase::Str,
                                text: WILD.to_string(),
                                outside: false,
                                ext_head: false,
                            }),
                            Err(e) => return Err(e),
                        }
                    }
                    vals
                }
                EnvPart::Arg(k) => self.arg(cx, *k, false, depth)?,
                EnvPart::ArgCall(k) => self.arg(cx, *k, true, depth)?,
                EnvPart::Call(k) => {
                    let Some(call) = cx.calls.get(*k as usize) else {
                        return Err("malformed fact".to_owned());
                    };
                    self.call(call, cx, depth)?
                }
            };
            let mut next = Vec::with_capacity(acc.len() * vals.len().max(1));
            for a in &acc {
                for v in &vals {
                    next.push(splice(a, v, i == 0)?);
                }
            }
            if next.len() > MAX_VALUES {
                return Err("too many alternatives".to_owned());
            }
            acc = next;
        }
        Ok(acc)
    }

    /// The value of argument `k` of the call being expanded (`returns`: what
    /// calling it returns).
    fn arg(&mut self, cx: Cx<'_>, k: u32, returns: bool, depth: u32) -> Result<Vec<Conc>, String> {
        let Some(bind) = cx.bind else {
            return Err("malformed fact".to_owned());
        };
        let Some(arg) = bind.call.args.get(k as usize) else {
            // Not passed: `undefined`, never a path.
            return Ok(Vec::new());
        };
        if returns {
            // A string argument cannot be called; an unknown one could return anything.
            if let Some(EnvAlt::Unknown(r)) =
                arg.value.iter().find(|a| matches!(a, EnvAlt::Unknown(_)))
            {
                return Err(format!("cannot evaluate `{r}`"));
            }
            return match &arg.returns {
                Some(r) => self.alts(r, bind.cx, depth + 1),
                None => Ok(Vec::new()),
            };
        }
        self.alts(&arg.value, bind.cx, depth + 1)
    }

    /// Every value of variable `name`: what the repository assigns and
    /// declares, and "set outside the repository".
    fn variable(&mut self, name: &str, depth: u32) -> Result<Vec<Conc>, String> {
        if !self.vars.iter().any(|v| v == name) {
            self.vars.push(name.to_owned());
        }
        let mut out = Vec::new();
        match known(name) {
            Some(Known::Root) => out.push(Conc {
                base: CBase::Dir(String::new()),
                text: String::new(),
                outside: false,
                ext_head: false,
            }),
            // Set outside the repository to a path outside it: no alternative.
            Some(Known::Outside) => {}
            None => out.push(Conc {
                base: CBase::Str,
                text: WILD.to_string(),
                outside: true,
                ext_head: true,
            }),
        }
        if let Some(values) = self.decls.get(name) {
            self.declared = true;
            for v in values {
                out.push(Conc {
                    base: CBase::Dir(String::new()),
                    text: glob_text(v),
                    outside: false,
                    ext_head: false,
                });
            }
        }
        if self.visiting.iter().any(|v| v == name) {
            return Ok(out);
        }
        let mut failed = None;
        // Command lines of tasks: `X=value cmd`.
        for t in self.tasks.get(name).map_or(&[][..], Vec::as_slice) {
            if let Some(d) = t.def
                && !self.sites.contains(&d)
            {
                self.sites.push(d);
            }
            let mut v = t.value.as_str();
            // The directory the command runs in, spelled out.
            for cwd in ["$PWD/", "${PWD}/", "$(pwd)/", "$INIT_CWD/", "${INIT_CWD}/"] {
                if let Some(rest) = v.strip_prefix(cwd) {
                    v = rest;
                    break;
                }
            }
            if v.contains(['$', '`']) {
                if failed.is_none() {
                    failed = Some(format!(
                        "{name} is assigned in {}: shell expansion `{}`",
                        t.task, t.value
                    ));
                }
                continue;
            }
            if v.is_empty() {
                continue;
            }
            let base = match &t.dir {
                Some(d) if !v.starts_with('/') => CBase::Dir(d.clone()),
                Some(_) => CBase::Str,
                None => CBase::Cwd,
            };
            let c = Conc {
                base,
                text: v.to_owned(),
                outside: false,
                ext_head: false,
            };
            if !out.contains(&c) {
                out.push(c);
            }
        }
        let Some(sites) = self.sets.get(name) else {
            return match failed {
                Some(e) => Err(e),
                None => Ok(out),
            };
        };
        self.visiting.push(name.to_owned());
        for s in sites.iter() {
            if !self.sites.contains(&s.node) {
                self.sites.push(s.node);
            }
            let cx = Cx {
                node: s.node,
                calls: &s.value.calls,
                bind: None,
            };
            // What the file's own facts cannot evaluate, a cross-file evaluator may.
            let asked = s.at.and_then(|at| (self.oracle)(NodeId(s.node), at));
            let evaluated = match asked {
                Some(answer) => {
                    for f in &answer.files {
                        if !self.sites.contains(&f.0) {
                            self.sites.push(f.0);
                        }
                    }
                    // The answer has no calls of its own: what it could not follow is
                    // unknown.
                    let acx = Cx {
                        node: s.node,
                        calls: &[],
                        bind: None,
                    };
                    self.alts(&answer.alts, acx, depth + 1)
                }
                None => self.alts(&s.value.alts, cx, depth + 1),
            };
            match evaluated {
                Ok(vals) => {
                    for v in vals {
                        // A variable assigned from a variable set outside: covered by this
                        // variable's own outside alternative.
                        if !out.contains(&v) {
                            out.push(v);
                        }
                    }
                }
                Err(e) if failed.is_none() => {
                    failed = Some(format!(
                        "{name} is assigned at {}:{}: {e}",
                        self.g.path_of(s.node as usize),
                        s.line
                    ));
                }
                Err(_) => {}
            }
        }
        self.visiting.pop();
        match failed {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    /// The result of `call`: the helper it names, with the call's arguments.
    fn call(&mut self, call: &EnvCall, cx: Cx<'_>, depth: u32) -> Result<Vec<Conc>, String> {
        let mut targets: Vec<u32> = Vec::new();
        for e in GraphView::out_edges(self.g, NodeId(cx.node)) {
            // An import also links the package's manifest: only source files can define
            // the function.
            if e.specifier == Some(call.module.as_str())
                && let Some(t) = e.to
                && GraphView::lang(self.g, t) == Some("js")
                && !targets.contains(&t.0)
            {
                targets.push(t.0);
            }
        }
        if targets.is_empty() {
            return Err(format!(
                "call {}#{}: the module is not in the graph",
                call.module, call.export
            ));
        }
        let mut out = Vec::new();
        for t in targets {
            let Some(helper) = self.helpers.get(&(t, call.export.as_str())) else {
                return Err(format!(
                    "call {}#{}: not an environment helper in {}",
                    call.module,
                    call.export,
                    self.g.path_of(t as usize)
                ));
            };
            let bind = Bind { call, cx };
            let hcx = Cx {
                node: t,
                calls: &helper.calls,
                bind: Some(&bind),
            };
            out.extend(self.alts(&helper.alts, hcx, depth + 1)?);
        }
        Ok(out)
    }
}

/// A declared path or glob as text with [`WILD`] for its wildcards.
fn glob_text(glob: &str) -> String {
    glob.replace("**", &WILD.to_string())
        .replace('*', &WILD.to_string())
}

/// Whether a use's value is the environment's: every alternative is fed by the
/// environment (a variable, or a call of a helper whose template reads one),
/// and none has a part that another function returns. Calls of other functions
/// are not this module's.
///
/// One record stands for every alternative of its origin, and a record this
/// layer keeps as an environment read is ignored by default when it is a file
/// read (an input from outside the repository). So one alternative that is not
/// the environment's keeps the record what it was: `readFileSync(c ?
/// getEnv('X', F) : pick())` is as open as `readFileSync(pick())`. (The
/// extractor's rule for the origin of a record is the same: `env` only when
/// every unknown part is one the policy may ignore.)
fn env_fed(
    alts: &[EnvAlt],
    cx: Cx<'_>,
    helpers: &HashMap<(u32, &str), &EnvValue>,
    g: &Graph,
) -> bool {
    let helper = |k: u32| {
        cx.calls.get(k as usize).is_some_and(|call| {
            GraphView::out_edges(g, NodeId(cx.node)).any(|e| {
                e.specifier == Some(call.module.as_str())
                    && e.to
                        .is_some_and(|t| helpers.contains_key(&(t.0, call.export.as_str())))
            })
        })
    };
    !alts.is_empty()
        && alts.iter().all(|a| match a {
            EnvAlt::Unknown(_) => false,
            EnvAlt::Term(t) => {
                let mut fed = false;
                for p in &t.parts {
                    match p {
                        EnvPart::Var(_) => fed = true,
                        EnvPart::Call(k) if helper(*k) => fed = true,
                        EnvPart::Call(_) => return false,
                        _ => {}
                    }
                }
                fed
            }
        })
}

/// `prefix` followed by `v`. At the head of a term the value brings its own
/// anchor.
fn splice(prefix: &Conc, v: &Conc, head: bool) -> Result<Conc, String> {
    if head && prefix.text.is_empty() {
        let absolute = v.text.starts_with('/') || v.text.starts_with(WILD);
        return match (&prefix.base, &v.base) {
            (CBase::Str, _) | (CBase::Cwd, CBase::Dir(_) | CBase::Cwd) => Ok(v.clone()),
            (CBase::Cwd, CBase::Str) => Ok(Conc {
                // `path.resolve(v)`: an absolute value stays, a relative one is relative to
                // the working directory.
                base: if absolute { CBase::Str } else { CBase::Cwd },
                text: v.text.clone(),
                outside: v.outside,
                ext_head: v.ext_head,
            }),
            // `path.join(__dirname, v)`: a relative value, or one set outside the
            // repository (unknown segments below the directory).
            (CBase::Dir(d), CBase::Str) if !v.text.starts_with('/') => Ok(Conc {
                base: CBase::Dir(d.clone()),
                text: v.text.clone(),
                outside: false,
                ext_head: false,
            }),
            (CBase::Dir(_), _) => Err("an absolute value joined to a file's directory".to_owned()),
        };
    }
    if v.base != CBase::Str {
        return Err("an anchored path inside a path".to_owned());
    }
    Ok(Conc {
        base: prefix.base.clone(),
        text: format!("{}{}", prefix.text, v.text),
        outside: false,
        ext_head: prefix.ext_head,
    })
}

/// What an evaluated value names.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    /// Nothing in the repository.
    Nothing,
    /// A repository path.
    Exact(String),
    /// Repository paths matching a glob.
    Glob(String),
    /// Repository paths that end with this path or glob (relative to an unknown
    /// working directory). With `true`: the unknown directory is a variable
    /// that only something outside the repository sets; when the suffix is
    /// too broad to bound anything, the path is taken to be outside the
    /// repository, like the bare variable.
    Suffix(String, bool),
    /// A module specifier relative to the place the load searches from: the
    /// loading file, unless the use says otherwise ([`EnvFrom`]).
    Relative(String),
}

/// Normalizes `text` (with [`WILD`] for unknown text) below `dir` into a
/// repository path or glob. `None`: it leaves the repository.
fn normalize(dir: &str, text: &str) -> Option<Result<String, String>> {
    let mut parts: Vec<String> = dir
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let mut wild = false;
    let segs: Vec<&str> = text.split('/').collect();
    for (i, seg) in segs.iter().copied().enumerate() {
        match seg {
            "" | "." => {}
            ".." => {
                if wild {
                    return Some(Err("`..` after unknown path segments".to_owned()));
                }
                parts.pop()?;
            }
            s if s.contains(WILD) => {
                wild = true;
                if s.chars().all(|c| c == WILD) {
                    parts.push("**".to_owned());
                } else {
                    // Unknown text at the end of a segment may also contain separators.
                    let pat = s.replace(WILD, "*");
                    if !s.ends_with(WILD) {
                        parts.push(pat);
                    } else if segs[i + 1..].iter().all(|r| r.is_empty() || *r == ".") {
                        // The last segment: the name itself, or anything below it.
                        parts.push(format!("{pat}{{,/**}}"));
                    } else {
                        parts.push(pat);
                        parts.push("**".to_owned());
                    }
                }
            }
            s => parts.push(s.to_owned()),
        }
    }
    Some(Ok(parts.join("/")))
}

impl Graph {
    /// What the evaluated value `c` names for a reference of `kind`: one
    /// target, or for a value that is an absolute path every file of the
    /// repository that its trailing components name. The code that sets the
    /// variable writes the path of the place it runs at (`process.env.DATA
    /// = '/app/data/x.json'`), and the tree does not say where that is: the
    /// repository seen from another place
    /// ([`turborepo_trace_core::fs::seen_from_elsewhere`]), the same wherever
    /// the checkout is. A reading that starts with unknown text names
    /// nothing (the mount point itself), and neither does one whose known
    /// part is no file or directory of the tree (`walked`, sorted): the
    /// layer is made again for a tree that has it.
    fn env_targets(&self, c: &Conc, module: bool, walked: &[&str]) -> Result<Vec<Target>, String> {
        let text = c.text.strip_prefix("file://").unwrap_or(&c.text);
        let absolute =
            !c.outside && matches!(c.base, CBase::Str | CBase::Cwd) && text.starts_with('/');
        if !absolute {
            return Ok(vec![self.env_target(c, module)?]);
        }
        let literal = text.replace(WILD, "");
        if is_glob(&literal) {
            return Err(format!("glob characters in `{literal}`"));
        }
        let mut out = Vec::new();
        for e in turborepo_trace_core::fs::seen_from_elsewhere(None, text) {
            let known: Vec<&str> = e.rel.split('/').take_while(|c| !c.contains(WILD)).collect();
            if known.is_empty() {
                continue;
            }
            let known = known.join("/");
            let below = format!("{known}/");
            let at = walked.partition_point(|f| *f < known.as_str());
            // A module path may name the file with an extension added.
            let with_extension = module && known == e.rel;
            let in_tree = walked[at..]
                .iter()
                .take_while(|f| f.starts_with(known.as_str()))
                .any(|f| {
                    *f == known
                        || f.starts_with(below.as_str())
                        || (with_extension && f[known.len()..].starts_with('.'))
                });
            if !in_tree {
                continue;
            }
            match normalize("", &e.rel) {
                None => {}
                Some(Err(e)) => return Err(e),
                Some(Ok(p)) if is_glob(&p) => out.push(Target::Glob(p)),
                Some(Ok(p)) => out.push(Target::Exact(p)),
            }
        }
        if out.is_empty() {
            out.push(Target::Nothing);
        }
        Ok(out)
    }

    /// What the evaluated value `c` names for a reference of `kind` (a value
    /// that is no absolute path: [`Graph::env_targets`]).
    fn env_target(&self, c: &Conc, module: bool) -> Result<Target, String> {
        if c.outside {
            // The bare variable, set outside the repository: a path outside it.
            return Ok(Target::Nothing);
        }
        let literal = c.text.replace(WILD, "");
        if is_glob(&literal) {
            return Err(format!("glob characters in `{literal}`"));
        }
        let from_dir = |dir: &str, text: &str| -> Result<Target, String> {
            match normalize(dir, text) {
                None => Ok(Target::Nothing),
                Some(Err(e)) => Err(e),
                Some(Ok(p)) if is_glob(&p) => Ok(Target::Glob(p)),
                Some(Ok(p)) => Ok(Target::Exact(p)),
            }
        };
        match &c.base {
            CBase::Dir(d) => from_dir(d, &c.text),
            CBase::Str | CBase::Cwd => {
                let t = c.text.as_str();
                if t.is_empty() {
                    return Ok(Target::Nothing);
                }
                let t = t.strip_prefix("file://").unwrap_or(t);
                if t.starts_with('/') {
                    // Absolute: [`Graph::env_targets`].
                    return Ok(Target::Nothing);
                }
                let scheme = t.split_once("://").is_some_and(|(s, _)| {
                    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '+')
                });
                if scheme || t.starts_with('~') || t.as_bytes().get(1) == Some(&b':') {
                    return Ok(Target::Nothing);
                }
                if module && c.base == CBase::Str && !t.starts_with(WILD) {
                    return if t.starts_with("./") || t.starts_with("../") {
                        Ok(Target::Relative(t.to_owned()))
                    } else {
                        Err(format!(
                            "module specifier `{t}` from the environment: packages are not \
                             resolved here"
                        ))
                    };
                }
                // Relative to a working directory this module does not know: leading
                // `..` and unknown directories are dropped, the rest is a path suffix.
                let ext = c.ext_head && t.starts_with(WILD);
                let mut rest = t;
                loop {
                    if let Some(r) = rest.strip_prefix("../") {
                        rest = r;
                    } else if let Some(r) = rest.strip_prefix("./") {
                        rest = r;
                    } else if let Some(r) = rest.strip_prefix(WILD) {
                        if r.is_empty() {
                            // Nothing but unknown text: below a directory that only
                            // something outside the repository names, it is outside too.
                            return if ext {
                                Ok(Target::Nothing)
                            } else {
                                Err("an unknown path".to_owned())
                            };
                        }
                        if !r.starts_with('/') && !r.starts_with(WILD) {
                            // Unknown text glued to a name (`${X}.json`).
                            return match normalize("", &format!("{WILD}{r}")) {
                                Some(Ok(p)) => Ok(Target::Suffix(p, ext)),
                                Some(Err(e)) => Err(e),
                                None => Ok(Target::Nothing),
                            };
                        }
                        rest = r.trim_start_matches('/');
                    } else {
                        break;
                    }
                }
                match normalize("", rest) {
                    None => Ok(Target::Nothing),
                    Some(Err(e)) => Err(e),
                    Some(Ok(p)) if p.is_empty() => Err("the working directory itself".to_owned()),
                    Some(Ok(p)) => Ok(Target::Suffix(p, ext)),
                }
            }
        }
    }

    /// Adds what `target` names for a reference of `kind` in file `reader` to
    /// `out`. `from`: where a module load searches from, which is what a
    /// relative specifier is resolved against.
    #[allow(clippy::too_many_arguments)]
    fn env_resolve<'w>(
        &self,
        walked: &[&'w str],
        names: &NameIndex<'w>,
        reader: u32,
        kind: RefKind,
        from: &EnvFrom,
        target: Target,
        out: &mut Resolution,
    ) -> Result<(), String> {
        let site = |spec: String| SiteRef {
            kind,
            attr: 0,
            specs: Some(vec![spec]),
            line: 0,
            unc_span: [0, 0],
            js: true,
        };
        let mut add = |res: Option<Resolution>, what: &str| -> Result<(), String> {
            let Some(res) = res else {
                return Err(format!("`{what}` cannot be resolved here"));
            };
            out.edges.extend(res.edges);
            out.probes.extend(res.probes);
            out.inputs.extend(res.inputs);
            out.members.extend(res.members);
            Ok(())
        };
        let module = is_module_kind(kind);
        match target {
            Target::Nothing => Ok(()),
            Target::Exact(p) => {
                if p.is_empty() {
                    return Err("the repository root".to_owned());
                }
                add(self.resolve_in_cwd(walked, "", &site(p.clone())), &p)
            }
            Target::Relative(rel) => {
                let own = dir_of(self.path_of(reader as usize));
                let dir = match from {
                    EnvFrom::File => own.to_owned(),
                    // A `require` function made for a file in another directory.
                    EnvFrom::Dir(d) => match normalize(own, d) {
                        Some(Ok(p)) if !is_glob(&p) => p,
                        _ => {
                            return Err(format!(
                                "`{rel}` is searched from `{d}`, which is not a directory of the \
                                 repository"
                            ));
                        }
                    },
                    // Resolving it from the file would be a guess.
                    EnvFrom::Unknown => {
                        return Err(format!(
                            "`{rel}` is relative to the place the `require` function searches \
                             from, which is not known"
                        ));
                    }
                };
                add(self.resolve_in_cwd(walked, &dir, &site(rel.clone())), &rel)
            }
            Target::Glob(_) if module => Err("a module path with unknown parts".to_owned()),
            Target::Suffix(p, _) if module && is_glob(&p) => {
                Err("a module path with unknown parts".to_owned())
            }
            // A literal module path below a directory that is not known here (the working
            // directory of whoever runs the loader): the module is resolved in every
            // directory of the repository that holds one of its candidates.
            Target::Suffix(p, _) if module => {
                let mut cands: Vec<String> = vec![p.clone()];
                cands.extend(MODULE_EXTS.iter().map(|e| format!("{p}{e}")));
                cands.extend(MODULE_EXTS.iter().map(|e| format!("{p}/index{e}")));
                cands.push(format!("{p}/package.json"));
                let by_name = names.get(walked);
                let mut dirs: Vec<&str> = Vec::new();
                for c in &cands {
                    let name = c.rsplit('/').next().unwrap_or(c);
                    let tail = format!("/{c}");
                    for &i in by_name.get(name).map_or(&[][..], Vec::as_slice) {
                        let f = walked[i as usize];
                        let dir = if f == c.as_str() {
                            ""
                        } else if let Some(d) = f.strip_suffix(tail.as_str()) {
                            d
                        } else {
                            continue;
                        };
                        if !dirs.contains(&dir) {
                            if dirs.len() == MAX_MODULE_DIRS {
                                return Err(format!(
                                    "`**/{p}` names modules in more than {MAX_MODULE_DIRS} \
                                     directories"
                                ));
                            }
                            dirs.push(dir);
                        }
                    }
                }
                for d in dirs {
                    add(self.resolve_in_cwd(walked, d, &site(p.clone())), &p)?;
                }
                // A candidate created in another directory.
                for pattern in [
                    format!("**/{p}"),
                    format!("**/{p}.*"),
                    format!("**/{p}/index.*"),
                    format!("**/{p}/package.json"),
                ] {
                    out.members.push((String::new(), pattern));
                }
                Ok(())
            }
            Target::Glob(p) => {
                let g = Glob::new(&p).map_err(|e| format!("`{p}`: {e}"))?;
                let (dir, pattern) = split_glob(&p);
                if kind == RefKind::FsReadDir {
                    out.members.push((dir_of(&p).to_owned(), "*".to_owned()));
                    out.members.push((dir, pattern));
                    return Ok(());
                }
                let prefix = if dir.is_empty() {
                    String::new()
                } else {
                    format!("{dir}/")
                };
                let lo = walked.partition_point(|f| *f < prefix.as_str());
                let mut n = 0;
                // A literal every match contains: a cheap test before the glob.
                let lit = crate::build::required_literal(&p).unwrap_or("");
                for f in walked[lo..]
                    .iter()
                    .take_while(|f| f.starts_with(prefix.as_str()))
                {
                    if f.contains(lit) && g.matches(f) {
                        n += 1;
                        if n > MAX_DIR_FILES {
                            return Err(format!("`{p}` matches too many files"));
                        }
                        match self.lookup(f) {
                            Some(x) => out.edges.push(x),
                            None => out.inputs.push((*f).to_owned()),
                        }
                    }
                }
                out.members.push((dir, pattern));
                Ok(())
            }
            Target::Suffix(p, outside_if_broad) => {
                let pattern = format!("**/{p}");
                if kind == RefKind::FsReadDir {
                    out.members.push((String::new(), format!("{pattern}/*")));
                    return Ok(());
                }
                let glob = is_glob(&p)
                    .then(|| Glob::new(&pattern).map_err(|e| format!("`{p}`: {e}")))
                    .transpose()?;
                let tail = format!("/{p}");
                // Too many matches bound nothing: the read stays open, unless the unknown
                // directory can only be set outside the repository.
                let mut hits: Vec<&str> = Vec::new();
                let mut broad = false;
                match &glob {
                    Some(g) => {
                        let lit = crate::build::required_literal(&p).unwrap_or("");
                        for f in walked {
                            if f.contains(lit) && g.matches(f) {
                                hits.push(f);
                                if hits.len() > MAX_DIR_FILES {
                                    broad = true;
                                    break;
                                }
                            }
                        }
                    }
                    // Files with the path's last name, then the whole suffix.
                    None => {
                        let name = p.rsplit('/').next().unwrap_or(&p);
                        for &i in names.get(walked).get(name).map_or(&[][..], Vec::as_slice) {
                            let f = walked[i as usize];
                            if f == p || f.ends_with(tail.as_str()) {
                                hits.push(f);
                                if hits.len() > MAX_DIR_FILES {
                                    broad = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if broad {
                    return if outside_if_broad {
                        Ok(())
                    } else {
                        Err(format!("`{pattern}` matches too many files"))
                    };
                }
                for f in hits {
                    match self.lookup(f) {
                        Some(x) => out.edges.push(x),
                        None => out.inputs.push(f.to_owned()),
                    }
                }
                out.members.push((String::new(), pattern));
                Ok(())
            }
        }
    }

    /// The decoded environment facts of the first-party files.
    fn env_facts(&self) -> Facts<'_> {
        let mut facts = Facts::default();
        for (ni, node) in self.nodes.iter().enumerate() {
            if node.shard == NONE {
                continue;
            }
            let a = self.shards[node.shard as usize].get();
            if a.package.kind != kind::FIRST_PARTY {
                continue;
            }
            let Some(ex) = a.files[node.file as usize].extraction.as_ref() else {
                continue;
            };
            if ex.facts.is_empty() {
                continue;
            }
            let mut decoded: Vec<EnvFact> = Vec::new();
            for f in ex.facts.iter() {
                let f = f.as_str();
                if let Some(name) = set_name(f) {
                    facts.sets.raw.entry(name).or_default().push((ni as u32, f));
                } else if f.starts_with("env:{\"set\"") {
                    // A name with escapes: no variable is written that way.
                } else if let Some(d) = EnvFact::decode(f) {
                    decoded.push(d);
                }
            }
            if !decoded.is_empty() {
                facts.files.push((ni as u32, decoded));
            }
        }
        facts
    }

    /// The resolution of references whose path comes from the environment (see
    /// the module documentation): apply it with
    /// [`turborepo_trace_graph::Patched`]. `decls` are the values
    /// declared in the hints file.
    pub fn env_patch(&self, decls: &[EnvDecl]) -> (Patch, EnvStats) {
        self.env_patch_with(decls, &|_, _| None)
    }

    /// [`Graph::env_patch_with`] through the store's cache of derived layers
    /// ([`Graph::cached_patch`]): the patch is a function of the graph and the
    /// declared values, so an unchanged repository reads it back. (An
    /// evaluator must answer for the graph's state, as [`EnvOracle`]
    /// requires, for this to hold with one.) With
    /// `TURBO_TRACE_DEBUG_ENV_REPORT` it is always computed. The last value:
    /// read from the cache.
    pub fn env_patch_cached(
        &self,
        decls: &[EnvDecl],
        oracle: &EnvOracle<'_>,
    ) -> (Patch, EnvStats, bool) {
        if turborepo_trace_core::debug::var_os("ENV_REPORT").is_some_and(|p| !p.is_empty()) {
            let (p, s) = self.env_patch_with(decls, oracle);
            return (p, s, false);
        }
        let mut h = xxhash_rust::xxh3::Xxh3::new();
        for d in decls {
            h.update(d.name.as_bytes());
            h.update(&[0]);
            for v in &d.values {
                h.update(v.as_bytes());
                h.update(&[1]);
            }
            h.update(&[2]);
        }
        let (patch, meta, hit) = self.cached_patch("env", h.digest128(), || {
            let (p, s) = self.env_patch_with(decls, oracle);
            let meta = [
                s.uses,
                s.resolved,
                s.kept,
                s.variables,
                s.assignments,
                s.helpers,
                s.edges,
            ]
            .map(|v| v as u64)
            .to_vec();
            (p, meta)
        });
        let m = |i: usize| meta.get(i).copied().unwrap_or(0) as usize;
        let stats = EnvStats {
            uses: m(0),
            resolved: m(1),
            kept: m(2),
            variables: m(3),
            assignments: m(4),
            helpers: m(5),
            edges: m(6),
        };
        (patch, stats, hit)
    }

    /// [`Graph::env_patch`] with a cross-file evaluator for the assigned
    /// expressions the per-file facts left unknown ([`EnvOracle`]).
    pub fn env_patch_with(&self, decls: &[EnvDecl], oracle: &EnvOracle<'_>) -> (Patch, EnvStats) {
        let mut patch = Patch::new();
        let mut stats = EnvStats::default();
        turborepo_trace_core::timing::mark("env: start");
        let facts = self.env_facts();
        turborepo_trace_core::timing::mark("env: facts");
        let sets = &facts.sets;
        let mut helpers: HashMap<(u32, &str), &EnvValue> = HashMap::new();
        let mut uses: Vec<UseSite<'_>> = Vec::new();
        for (node, fs) in &facts.files {
            for f in fs {
                match f {
                    EnvFact::Set { .. } => {}
                    EnvFact::Helper { export, value } => {
                        helpers.insert((*node, export.as_str()), value);
                    }
                    EnvFact::Use {
                        span,
                        line,
                        kind,
                        origin,
                        value,
                        from,
                        shared,
                    } => {
                        let Some(&kind) = RefKind::ALL.get(*kind as usize) else {
                            continue;
                        };
                        uses.push(UseSite {
                            node: *node,
                            span: *span,
                            line: *line,
                            kind,
                            origin: *origin,
                            value,
                            from,
                            shared: *shared,
                        });
                    }
                }
            }
        }
        stats.uses = uses.len();
        stats.helpers = helpers.len();
        if uses.is_empty() {
            return (patch, stats);
        }
        let mut declared: HashMap<&str, Vec<&str>> = HashMap::new();
        for d in decls {
            declared
                .entry(d.name.as_str())
                .or_default()
                .extend(d.values.iter().map(String::as_str));
        }
        let walked_buf = self.walked_files();
        let walked: Vec<&str> = walked_buf
            .as_deref()
            .map(|b| crate::manifest::files_in(b).collect())
            .unwrap_or_default();
        let names = NameIndex::default();
        let task_sets = task_sets(self);
        turborepo_trace_core::timing::mark("env: walked files");
        let mut report = turborepo_trace_core::debug::var_os("ENV_REPORT")
            .filter(|p| !p.is_empty())
            .map(|_| Vec::<serde_json::Value>::new());
        let mut all_vars: HashSet<String> = HashSet::new();
        for u in &uses {
            let started = std::time::Instant::now();
            let mut ev = Eval {
                g: self,
                tasks: &task_sets,
                oracle,
                sets,
                helpers: &helpers,
                decls: &declared,
                visiting: Vec::new(),
                sites: Vec::new(),
                vars: Vec::new(),
                declared: false,
            };
            let cx = Cx {
                node: u.node,
                calls: &u.value.calls,
                bind: None,
            };
            let module = is_module_kind(u.kind);
            let mut res = Resolution::default();
            let mut values: Vec<String> = Vec::new();
            let outcome: Result<(), String> = (|| {
                if walked_buf.is_none() {
                    return Err("the snapshot has no file list".to_owned());
                }
                let concs = ev.alts(&u.value.alts, cx, 0)?;
                if module && !ev.declared && !concs.is_empty() && concs.iter().all(|c| c.outside) {
                    // Code named only from outside the repository: nothing bounds it.
                    return Err(format!(
                        "the repository never assigns {}: the module is named outside it",
                        ev.vars.join(", ")
                    ));
                }
                for c in &concs {
                    for target in self.env_targets(c, module, &walked)? {
                        if report.is_some() {
                            values.push(format!("{target:?}"));
                        }
                        let (node, kind) = (u.node, u.kind);
                        self.env_resolve(&walked, &names, node, kind, u.from, target, &mut res)?;
                    }
                }
                Ok(())
            })();
            let from = NodeId(u.node);
            // What was found is a dependency either way.
            if !res.edges.is_empty() {
                let label = patch.intern(&format!(
                    "env {}: {}:{}",
                    ev.vars.join(", "),
                    self.path_of(u.node as usize),
                    u.line
                ));
                let attr = EdgeAttr(crate::convert::edge_attr(
                    u.kind,
                    Phase::Runtime,
                    RefFlags::empty(),
                ));
                res.edges.sort_unstable();
                res.edges.dedup();
                for &t in &res.edges {
                    if t != u.node {
                        patch.edge_id(from, NodeId(t), attr, label);
                        stats.edges += 1;
                    }
                }
            }
            for p in &res.probes {
                patch.probe(from, p);
            }
            for p in &res.inputs {
                patch.input(from, p);
            }
            for (d, p) in &res.members {
                patch.membership(from, d, p);
            }
            // Editing an assignment of a variable the use reads re-runs the readers.
            for &s in &ev.sites {
                if s != u.node {
                    patch.input(from, self.path_of(s as usize));
                }
            }
            // The records of the use: resolved ones go; one that stays and is fed by the
            // environment is relabelled as an environment read with the reason, whatever
            // origin the extractor gave it (a call of a helper), so that one layer owns it.
            let origin = crate::convert::query_origin(u.origin);
            let span = q::Span {
                start: u.span[0],
                end: u.span[1],
            };
            let records: Vec<(q::UncertaintyKind, u32, Option<q::Span>, String, q::Effect)> =
                GraphView::uncertainties(self, from)
                    // An aliased loader keeps its record: the value is known, the loader
                    // is assumed.
                    .filter(|r| {
                        r.span == Some(span)
                            && r.origin == origin
                            && !r.is_covered()
                            && matches!(
                                r.kind,
                                q::UncertaintyKind::EnvDependent
                                    | q::UncertaintyKind::ComputedSpecifier
                            )
                    })
                    .map(|r| (r.kind, r.line, r.span, r.detail.to_owned(), r.effect))
                    .collect();
            // A patch names a record by kind, line and span, and a reference has one
            // record per origin of its unknown alternatives: next to the value of an
            // environment helper (origin `call`) a parameter, an imported binding, the
            // command line or a value nothing knows has a record of the same kind at the
            // same place. Dropping the key for this use would drop that record too and
            // close an alternative nothing resolved (`require(c ? getEnv('X', F) : p)`),
            // so a key that a record of another origin has is shared, whatever the use
            // says: for a load asked of its callee the records all end as `unknown`, and
            // only the use can say so.
            let shared = u.shared
                || GraphView::uncertainties(self, from).any(|r| {
                    r.origin != origin
                        && records.iter().any(|(kind, line, span, ..)| {
                            (*kind, *line, *span) == (r.kind, r.line, r.span)
                        })
                });
            match &outcome {
                // A record that other alternatives of the reference share (they have the
                // same kind, line and span, which is all a patch can name) stays as it
                // is: what was found above is a dependency, and the reference is open by
                // the other alternative either way.
                _ if shared => {}
                Ok(()) => {
                    for (kind, line, span, _, _) in records {
                        patch.drop_uncertainty(from, kind, line, span);
                    }
                    stats.resolved += 1;
                }
                Err(reason) if env_fed(&u.value.alts, cx, &helpers, self) => {
                    for (kind, line, span, detail, effect) in records {
                        patch.drop_uncertainty(from, kind, line, span);
                        patch.add_uncertainty_from(
                            from,
                            q::UncertaintyKind::EnvDependent,
                            line,
                            &format!("{detail}; {reason}"),
                            effect,
                            q::Origin::Env,
                        );
                    }
                    stats.kept += 1;
                }
                Err(_) => {}
            }
            if let Some(rep) = &mut report {
                rep.push(serde_json::json!({
                    "file": self.path_of(u.node as usize),
                    "micros": started.elapsed().as_micros() as u64,
                    "line": u.line,
                    "kind": u.kind.as_str(),
                    "origin": crate::convert::query_origin(u.origin).as_str(),
                    "variables": ev.vars,
                    "resolved": outcome.is_ok(),
                    "shared": shared,
                    "reason": outcome.as_ref().err(),
                    "values": values,
                    "edges": res.edges.len(),
                    "probes": res.probes.len(),
                    "inputs": res.inputs.len(),
                    "memberships": res.members.len(),
                    "assignment_files": ev.sites.iter().map(|&s| self.path_of(s as usize)).collect::<Vec<_>>(),
                }));
            }
            all_vars.extend(ev.vars);
        }
        turborepo_trace_core::timing::mark("env: uses");
        stats.variables = all_vars.len();
        stats.assignments = all_vars
            .iter()
            .map(|v| sets.count(v) + task_sets.get(v).map_or(0, Vec::len))
            .sum();
        if let (Some(rep), Some(path)) = (report, turborepo_trace_core::debug::var_os("ENV_REPORT"))
        {
            let mut text = String::new();
            for r in rep {
                text.push_str(&r.to_string());
                text.push('\n');
            }
            let _ = std::fs::write(path, text);
        }
        (patch, stats)
    }
}

/// Splits a glob into its static directory and the pattern below it.
fn split_glob(p: &str) -> (String, String) {
    let segs: Vec<&str> = p.split('/').collect();
    let k = segs.iter().position(|s| is_glob(s)).unwrap_or(segs.len());
    (segs[..k].join("/"), segs[k..].join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conc(base: CBase, text: &str) -> Conc {
        Conc {
            base,
            text: text.replace('\u{1}', "\u{0}"),
            outside: false,
            ext_head: false,
        }
    }

    #[test]
    fn normalizes_paths_and_globs() {
        assert_eq!(
            normalize("a/b", "../c/./d.json"),
            Some(Ok("a/c/d.json".to_owned()))
        );
        assert_eq!(normalize("a", "../../x"), None);
        assert_eq!(
            normalize("a", "\u{0}/x.json"),
            Some(Ok("a/**/x.json".to_owned()))
        );
        assert_eq!(
            normalize("a", "pre-\u{0}.json"),
            Some(Ok("a/pre-*.json".to_owned()))
        );
        assert_eq!(
            normalize("a", "dir-\u{0}"),
            Some(Ok("a/dir-*{,/**}".to_owned()))
        );
        assert_eq!(
            normalize("a", "dir-\u{0}/x.json"),
            Some(Ok("a/dir-*/**/x.json".to_owned()))
        );
        let g = Glob::new("a/dir-*{,/**}").unwrap();
        assert!(g.matches("a/dir-x") && g.matches("a/dir-x/y/z.json") && !g.matches("a/other"));
        assert_eq!(
            split_glob("a/dir-*{,/**}"),
            ("a".to_owned(), "dir-*{,/**}".to_owned())
        );
        assert_eq!(
            glob_text("fixtures/**/a-*.json"),
            "fixtures/\u{0}/a-\u{0}.json"
        );
        assert!(matches!(normalize("a", "\u{0}/../x"), Some(Err(_))));
        assert_eq!(
            split_glob("a/b/**/x.json"),
            ("a/b".to_owned(), "**/x.json".to_owned())
        );
    }

    #[test]
    fn splices_values() {
        let head = conc(CBase::Str, "");
        let abs = conc(CBase::Str, "/var/run/token");
        let rel = conc(CBase::Str, "dev/flags.json");
        let anchored = conc(CBase::Dir("pkg".into()), "fixtures/a.json");
        assert_eq!(splice(&head, &abs, true), Ok(abs.clone()));
        assert_eq!(splice(&head, &anchored, true), Ok(anchored.clone()));
        // `path.resolve(v)`.
        let cwd = conc(CBase::Cwd, "");
        assert_eq!(splice(&cwd, &abs, true), Ok(abs.clone()));
        assert_eq!(
            splice(&cwd, &rel, true),
            Ok(conc(CBase::Cwd, "dev/flags.json"))
        );
        // After a prefix only plain text can follow.
        let pre = conc(CBase::Str, "config/");
        assert_eq!(
            splice(&pre, &conc(CBase::Str, "test"), false),
            Ok(conc(CBase::Str, "config/test"))
        );
        assert!(splice(&pre, &anchored, false).is_err());
        assert!(splice(&conc(CBase::Dir("p".into()), ""), &abs, true).is_err());
    }
}
