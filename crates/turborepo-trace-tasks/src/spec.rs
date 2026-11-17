// API-READY
//! The task data model: [`TaskId`], [`TaskSpec`] and [`Command`].
//!
//! Every path in a [`TaskSpec`] is absolute (lexically normalized, see
//! [`turborepo_trace_frameworks::fsx::normalize`]); the graph store turns them
//! into repo-relative paths. A path need not exist: a missing input is a probe
//! (creating it changes the task), a missing output is simply not produced yet.
//!
//! A spec is a *union*: recognizers, task sources and declared configuration
//! (turbo `inputs`) all add to the same spec, and nothing is ever removed.
//! Lists are deduplicated and sorted by [`TaskSpec::normalize`] so output is
//! deterministic.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use turborepo_trace_core::UncertaintyKind;

/// Identifies a task across the repo.
///
/// The string form ([`fmt::Display`], [`TaskId::parse`]) is what users type and
/// what the store keys on:
///
/// | Variant | String form | Example |
/// |---|---|---|
/// | [`TaskId::Script`] | `task:<pkg>#<name>` | `task:apps/web#build` |
/// | [`TaskId::Make`] | `make:<dir>:<target>` | `make:services/svc-a:dist` |
/// | [`TaskId::Docker`] | `docker:<path>[#stage]` | `docker:apps/web/Dockerfile#runner` |
/// | [`TaskId::Other`] | `<kind>:<rest>` | `just:.:test`, `gha:.github/workflows/ci.yml#test` |
///
/// `<pkg>`, `<dir>` and `<path>` are repo-relative with `/` separators; the
/// repo root is `.`. Package script and turbo tasks share [`TaskId::Script`]
/// (turbo's `pkg#task`). Sources other than scripts, make and docker use
/// [`TaskId::Other`] with their own `kind` (lowercase, no `:`), for example
/// `nx`, `just`, `taskfile`, `tox`, `nox`, `cargo`, `gha`, `shell`,
/// `pypackaging`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum TaskId {
    /// A package script or turbo task: `task:<pkg>#<name>`.
    Script {
        /// Package directory, repo-relative (`.` for the root).
        package: String,
        /// Script / task name.
        name: String,
    },
    /// A Makefile target: `make:<dir>:<target>`.
    Make {
        /// Directory of the Makefile, repo-relative.
        dir: String,
        /// Target name.
        target: String,
    },
    /// A Docker image build: `docker:<path>[#stage]`.
    Docker {
        /// The Dockerfile (or compose file), repo-relative.
        path: String,
        /// A build stage (`FROM … AS stage`) or compose service.
        stage: Option<String>,
    },
    /// Any other task source: `<kind>:<rest>`.
    Other {
        /// Source kind (`nx`, `just`, ...); lowercase ASCII, no `:`.
        kind: String,
        /// Source-defined remainder (conventionally `<dir>:<name>` or
        /// `<path>#<name>`).
        rest: String,
    },
}

impl TaskId {
    /// `task:<package>#<name>`.
    pub fn script(package: impl Into<String>, name: impl Into<String>) -> TaskId {
        TaskId::Script {
            package: package.into(),
            name: name.into(),
        }
    }

    /// `make:<dir>:<target>`.
    pub fn make(dir: impl Into<String>, target: impl Into<String>) -> TaskId {
        TaskId::Make {
            dir: dir.into(),
            target: target.into(),
        }
    }

    /// `docker:<path>[#stage]`.
    pub fn docker(path: impl Into<String>, stage: Option<String>) -> TaskId {
        TaskId::Docker {
            path: path.into(),
            stage,
        }
    }

    /// `<kind>:<rest>`.
    pub fn other(kind: impl Into<String>, rest: impl Into<String>) -> TaskId {
        TaskId::Other {
            kind: kind.into(),
            rest: rest.into(),
        }
    }

    /// The kind prefix: `task`, `make`, `docker`, or the [`TaskId::Other`]
    /// kind.
    pub fn kind(&self) -> &str {
        match self {
            TaskId::Script { .. } => "task",
            TaskId::Make { .. } => "make",
            TaskId::Docker { .. } => "docker",
            TaskId::Other { kind, .. } => kind,
        }
    }

    /// The string form without the `<kind>:` prefix (`apps/web#build`), which
    /// is what `--tasks` globs match.
    pub fn short(&self) -> String {
        let s = self.to_string();
        let k = self.kind().len() + 1;
        s[k..].to_string()
    }

    /// Parses the string form. Returns `None` without a `<kind>:` prefix or
    /// with an empty part.
    pub fn parse(s: &str) -> Option<TaskId> {
        let (kind, rest) = s.split_once(':')?;
        if kind.is_empty() || rest.is_empty() {
            return None;
        }
        match kind {
            "task" => {
                let (pkg, name) = rest.rsplit_once('#')?;
                (!pkg.is_empty() && !name.is_empty()).then(|| TaskId::script(pkg, name))
            }
            "make" => {
                let (dir, target) = rest.split_once(':')?;
                (!dir.is_empty() && !target.is_empty()).then(|| TaskId::make(dir, target))
            }
            "docker" => Some(match rest.rsplit_once('#') {
                Some((p, st)) if !p.is_empty() && !st.is_empty() => {
                    TaskId::docker(p, Some(st.to_string()))
                }
                _ => TaskId::docker(rest, None),
            }),
            k if k.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
            }) =>
            {
                Some(TaskId::other(k, rest))
            }
            _ => None,
        }
    }

    /// The package directory for script tasks, the directory for make tasks,
    /// the Dockerfile's directory for docker tasks; `None` for
    /// [`TaskId::Other`].
    pub fn dir(&self) -> Option<&str> {
        match self {
            TaskId::Script { package, .. } => Some(package),
            TaskId::Make { dir, .. } => Some(dir),
            TaskId::Docker { path, .. } => Some(path.rsplit_once('/').map_or(".", |(d, _)| d)),
            TaskId::Other { .. } => None,
        }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskId::Script { package, name } => write!(f, "task:{package}#{name}"),
            TaskId::Make { dir, target } => write!(f, "make:{dir}:{target}"),
            TaskId::Docker { path, stage: None } => write!(f, "docker:{path}"),
            TaskId::Docker {
                path,
                stage: Some(s),
            } => write!(f, "docker:{path}#{s}"),
            TaskId::Other { kind, rest } => write!(f, "{kind}:{rest}"),
        }
    }
}

impl From<TaskId> for String {
    fn from(t: TaskId) -> String {
        t.to_string()
    }
}

impl TryFrom<String> for TaskId {
    type Error = String;
    fn try_from(s: String) -> Result<TaskId, String> {
        TaskId::parse(&s).ok_or_else(|| format!("invalid task id `{s}`"))
    }
}

/// The language a [`TaskEntry`] is traced with.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryLang {
    /// JavaScript / TypeScript (node, tsx, ts-node, bun, deno, configs written
    /// in JS).
    Js,
    /// Python.
    Python,
    /// Go (a file or package directory passed to `go run`).
    Go,
    /// Rust (a crate root, e.g. an xtask `main.rs`).
    Rust,
    /// Let the store pick by extension (any registered language).
    Auto,
}

impl EntryLang {
    /// Guesses the language of `path` by extension.
    pub fn of(path: &Path) -> EntryLang {
        match path.extension().and_then(|e| e.to_str()) {
            Some("js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx") => EntryLang::Js,
            Some("py") => EntryLang::Python,
            Some("go") => EntryLang::Go,
            Some("rs") => EntryLang::Rust,
            _ => EntryLang::Auto,
        }
    }
}

/// A file traced with the language plugins as part of a task: its whole closure
/// (and its fs reads) become task inputs.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskEntry {
    /// The file (absolute). A directory for `go run ./cmd/x`.
    pub path: PathBuf,
    /// The language.
    pub lang: EntryLang,
}

/// A directory listing filtered by a glob: every file matching `pattern` under
/// `dir` (and not matching any of `exclude`) is an input; adding or deleting
/// one changes the task.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Membership {
    /// The directory (absolute).
    pub dir: PathBuf,
    /// Glob relative to `dir`, `/`-separated; `**` covers subdirectories.
    pub pattern: String,
    /// Globs relative to `dir` that are excluded (turbo `!negations`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Explanation (`turbo inputs`, `tsconfig include`, `unknown command`).
    pub reason: String,
    /// The task provably only reads the contents of the matching files: it
    /// neither runs them nor writes code made from them. True for declared
    /// hash inputs (turbo `inputs` and `globalDependencies`), for the
    /// sources of a tsconfig project that `tsc` only type-checks (`noEmit`,
    /// `emitDeclarationOnly`), and for the files a linter reads.
    ///
    /// False, the default, for everything else: a command may run the files (a
    /// test runner and its test files), compile, bundle or copy them into
    /// its outputs (`tsc` emitting JavaScript: the importer of the output
    /// runs a copy), or do either without the recognizer knowing (the
    /// package directory of an unknown or partly recognized command is the
    /// conservative fallback for exactly that).
    ///
    /// The graph store makes the task's edge to the hub of a read-only
    /// membership a data edge: a change of a member is a change for the
    /// task, but what a member would load when it runs (its uncertainty) is
    /// not the task's or its consumers' concern.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub data: bool,
    /// Only the files of the directory's own package: files of a workspace
    /// package nested below `dir` are not members. Set on the whole-package
    /// fallback of a command in the root package of a workspace
    /// ([`crate::fallback`]): the root package's files are the root minus
    /// the member directories.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
}

impl Membership {
    /// A membership with no excludes.
    pub fn new(
        dir: impl Into<PathBuf>,
        pattern: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Membership {
            dir: dir.into(),
            pattern: pattern.into(),
            exclude: Vec::new(),
            reason: reason.into(),
            data: false,
            own: false,
        }
    }

    /// The membership's identity: its files, whatever the reason and the way
    /// the task uses them. Memberships with the same key share one hub in
    /// the graph.
    pub fn key(&self) -> Membership {
        Membership {
            reason: String::new(),
            data: false,
            ..self.clone()
        }
    }
}

/// Why a task writes an [`Output`], and therefore whether the output takes part
/// in generated-file linking (an importer of the output depends on the task's
/// inputs).
///
/// Only outputs that produce files other code reads link to their producer
/// ([`OutputKind::links_producer`]):
///
/// | Kind | Set by | Links |
/// |---|---|---|
/// | [`Generated`](Self::Generated) | code generator recognizers and specs (`recognizers/codegen`, `specs/*.toml`), including committed generated files and their `>` redirections | yes |
/// | [`Declared`](Self::Declared) | declared outputs: turbo `outputs`, Taskfile `generates`, nx `outputs`, Make file targets | yes |
/// | [`Artifact`](Self::Artifact) | build tools (bundlers, `tsc`, compilers, test reports), and generic writes ([`TaskSpec::write`]) to a path that is not an existing source file | yes |
/// | [`Persistent`](Self::Persistent) | every output of a turbo `persistent: true` task (`tsc -b -w`, dev servers) | no |
/// | [`CacheOnly`](Self::CacheOnly) | turbo `outputs` outside the declaring package or inside another workspace package (`$TURBO_ROOT$/**/dist/**`): a cache-restore list | no |
/// | [`InPlace`](Self::InPlace) | generic writes (`cp`/`mv` destinations, `>` redirections, `touch`) to an existing source file, formatters with `--write`/`--fix` | no |
/// | [`CiSideEffect`](Self::CiSideEffect) | everything written by CI and container tasks (`gha:` and `docker:` tasks, `docker build --iidfile`/`--metadata-file`) | no |
/// | [`Unplaced`](Self::Unplaced) | a write to a path that is not a known place in the repository: outside it (`/tmp/report.txt`), or with a variable the shell parser could not expand (`$BUILD_DIR/pnpm-workspace.yaml`) | listings only, as listing-only links |
///
/// An in-place edit rewrites a tracked file: tests read the committed content,
/// so the edit is a later commit, not a file produced for them. A CI job's
/// writes become a later commit too, and a docker build produces an image, not
/// repository files. An unplaced write names no repository file: it stands for
/// a directory literally called `$BUILD_DIR` at the place the command runs,
/// which no importer can name. Only listings of the directories above it match,
/// so every reader of a root-wide listing is a consumer of the
/// task ([`unplaced`]). The graph store keeps those links as listing-only ones,
/// which build-task entries do not follow ([`Output::links_listings_only`]).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    /// Written by a code generator.
    Generated,
    /// Declared by the task's configuration.
    Declared,
    /// A build artifact (the default for [`TaskSpec::output`]).
    #[default]
    Artifact,
    /// Written by a persistent task (turbo `persistent: true`: `tsc -b -w`, a
    /// dev server): turbo rejects dependencies on persistent tasks and
    /// builds don't run them, so nothing built or tested in CI reads what
    /// they write.
    Persistent,
    /// A declared output outside the declaring task's package, or inside
    /// another workspace package: turbo restores it from the cache (a task
    /// collecting its dependencies' `dist/`), but the tasks of the package
    /// that contains it produce it.
    CacheOnly,
    /// An in-place edit of an existing source file.
    InPlace,
    /// A side effect of a CI or container task.
    CiSideEffect,
    /// A write to a path outside the repository, or to one with an unexpanded
    /// variable.
    Unplaced,
}

impl OutputKind {
    /// Whether an output of this kind links importers to its producing task.
    pub fn links_producer(self) -> bool {
        !matches!(
            self,
            OutputKind::Persistent
                | OutputKind::CacheOnly
                | OutputKind::InPlace
                | OutputKind::CiSideEffect
                | OutputKind::Unplaced
        )
    }

    /// The stable name (`in_place`).
    pub fn as_str(self) -> &'static str {
        match self {
            OutputKind::Generated => "generated",
            OutputKind::Declared => "declared",
            OutputKind::Artifact => "artifact",
            OutputKind::Persistent => "persistent",
            OutputKind::CacheOnly => "cache_only",
            OutputKind::InPlace => "in_place",
            OutputKind::CiSideEffect => "ci_side_effect",
            OutputKind::Unplaced => "unplaced",
        }
    }
}

/// Top-level directories (relative to the repository root, or any path
/// component below it) that hold build outputs. A write below one is an
/// [`OutputKind::Artifact`] even when the file exists.
pub const OUTPUT_DIRS: &[&str] = &[
    "dist",
    "build",
    "out",
    "output",
    ".next",
    ".nuxt",
    ".output",
    ".svelte-kit",
    ".turbo",
    ".vercel",
    ".cache",
    "coverage",
    ".nyc_output",
    "storybook-static",
    "target",
    "node_modules",
    "tmp",
    ".tmp",
];

/// The kind of a write by a generic command (`cp`/`mv` destination, `>`
/// redirection, `touch`) to `path`: [`OutputKind::InPlace`] when `path` is an
/// existing file inside `repo_root` and not below one of the [`OUTPUT_DIRS`],
/// else [`OutputKind::Artifact`].
///
/// This approximates "tracked and not gitignored": no gitignore matcher is
/// reachable from this crate through [`turborepo_trace_frameworks::fsx`] (the
/// store's walk has one, but the store depends on this crate), and reading
/// `.gitignore` files directly would bypass the recorded filesystem. The
/// existence check goes through `fsx`, so it is recorded as a probe of the
/// task: creating or deleting the file re-analyzes the task.
pub fn write_kind(path: &Path, repo_root: &Path) -> OutputKind {
    let Ok(rel) = path.strip_prefix(repo_root) else {
        return OutputKind::Artifact;
    };
    let in_output_dir = rel
        .parent()
        .into_iter()
        .flat_map(|p| p.components())
        .any(|c| OUTPUT_DIRS.iter().any(|d| c.as_os_str() == *d));
    if !in_output_dir && turborepo_trace_frameworks::fsx::is_file(path) {
        OutputKind::InPlace
    } else {
        OutputKind::Artifact
    }
}

/// Whether `path` (absolute), written by a task, is not a known place in the
/// repository: it lies outside `repo_root`, or one of its components holds a
/// shell variable or command substitution that was not expanded (`$OUT`,
/// `${OUT}`, `$(pwd)`, a backtick).
///
/// Where the task writes is then unknown. An importer of a file that the task
/// generates at the real location is not linked to it either way: the link is
/// made for the literal path. Such an importer keeps what it has without the
/// link: the committed file, or an unresolved reference covered by the paths
/// its resolution probed.
///
/// The test is on the path's text alone (no file system lookup: it runs outside
/// the task's recording frame, and a probe here would be one of every task of
/// the package). A file really named `$id.ts` that a task writes is taken for
/// unplaced too; its links are then listing-only.
pub fn unplaced(path: &Path, repo_root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(repo_root) else {
        return true;
    };
    rel.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        c.contains('`')
            || c.match_indices('$').any(|(i, _)| {
                c[i + 1..]
                    .chars()
                    .next()
                    .is_some_and(|n| n == '{' || n == '(' || n == '_' || n.is_ascii_alphanumeric())
            })
    })
}

/// Something a task writes.
///
/// `pattern: None` means `path` itself is the output: a file, or a directory
/// and everything below it. With a pattern, the outputs are the files under
/// `path` matching the glob (turbo `outputs: ["dist/**", "!dist/cache/**"]`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Output {
    /// File or directory (absolute).
    pub path: PathBuf,
    /// Glob relative to `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Excluded globs relative to `path`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Why the task writes it ([`OutputKind`]); decides whether it links to the
    /// task.
    #[serde(default)]
    pub kind: OutputKind,
    /// The [`TaskPart`] (by its [`TaskPart::path`]) that alone produces this
    /// output: importers link to the part's inputs instead of the whole
    /// task's. `None`: the task produces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PathBuf>,
}

impl Output {
    /// A file or directory output ([`OutputKind::Artifact`]).
    pub fn path(path: impl Into<PathBuf>) -> Output {
        Output::of_kind(path, OutputKind::Artifact)
    }

    /// A file or directory output of kind `kind`.
    pub fn of_kind(path: impl Into<PathBuf>, kind: OutputKind) -> Output {
        Output {
            path: path.into(),
            pattern: None,
            exclude: Vec::new(),
            kind,
            part: None,
        }
    }

    /// Whether the graph store links importers of this output to the producing
    /// task (generated-file linking). False for in-place edits and CI side
    /// effects.
    pub fn links_producer(&self) -> bool {
        self.kind.links_producer()
    }

    /// Whether the output links only as a listing-only link
    /// ([`OutputKind::Unplaced`]): the graph store links the readers of
    /// listings above it with a mark that build-task entries skip, and
    /// nothing else.
    pub fn links_listings_only(&self) -> bool {
        self.kind == OutputKind::Unplaced
    }

    /// Whether `file` (absolute) is one of these outputs.
    pub fn covers(&self, file: &Path) -> bool {
        let Ok(rel) = file.strip_prefix(&self.path) else {
            return false;
        };
        let rel = rel.to_string_lossy();
        let rel = rel.as_ref();
        let inc = match &self.pattern {
            None => true,
            Some(p) => crate::glob::matches(p, rel),
        };
        inc && !self.exclude.iter().any(|e| crate::glob::matches(e, rel))
    }
}

/// A package a task writes into its ecosystem's install location (Prisma's
/// default client `@prisma/client` and `.prisma/client` in `node_modules`).
///
/// The graph store links to the task every node that is the package:
/// - its opaque graph node (`node_modules/<name>@<version>`);
/// - `<dir>/node_modules/<name>` and everything below it, for every `dir` from
///   the package root (or [`OutputPackage::dir`]) up to the repository root.
///   That covers installed files, probes of an uninstalled package, and
///   listings;
/// - unresolved bare imports of `<name>` or `<name>/…` by the linked nodes. The
///   link replaces their `unresolved_specifier` uncertainty.
///
/// Only the `npm` ecosystem is linked so far.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutputPackage {
    /// `npm`.
    pub ecosystem: String,
    /// Package name (`@prisma/client`, `.prisma/client`).
    pub name: String,
    /// Where resolution starts, when not the task's package root (absolute).
    /// For example, the directory where a generator found the package
    /// installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
}

impl OutputPackage {
    /// The `node_modules` directories that hold the package for a task of the
    /// package at `package_root`: `<d>/node_modules/<name>` for every `d`
    /// from the start directory up to `repo_root` (both inclusive).
    pub fn locations(&self, package_root: &Path, repo_root: &Path) -> Vec<PathBuf> {
        let start = self.dir.as_deref().unwrap_or(package_root);
        let mut out = Vec::new();
        let mut d = Some(start);
        while let Some(x) = d {
            if !x.starts_with(repo_root) {
                break;
            }
            out.push(x.join("node_modules").join(&self.name));
            if x == repo_root {
                break;
            }
            d = x.parent();
        }
        out
    }

    /// Whether `file` (absolute) is in the package (see
    /// [`OutputPackage::locations`]).
    pub fn covers(&self, file: &Path, package_root: &Path, repo_root: &Path) -> bool {
        self.locations(package_root, repo_root)
            .iter()
            .any(|l| file.starts_with(l))
    }
}

/// A self-contained unit of work inside a task whose outputs are a function of
/// its own inputs only, whichever task runs it: one tsconfig project of a `tsc
/// -b` build.
///
/// `tsc -b` at the root of a monorepo builds every referenced project, and so
/// does the `tsc -b` of every package that references them. Each of those tasks
/// writes `packages/lib/dist`, but what lands there depends only on
/// `packages/lib`'s project (its config chain, its sources, the projects it
/// references), not on the other hundred projects of the solution. An
/// [`Output`] tagged with a part ([`Output::part`]) therefore
/// links its importers to the part: they are affected by the part's inputs, not
/// by every input of every task that happens to build the project.
///
/// The part's inputs are also inputs of the task (the task spec stays the
/// union). The graph store gives every part one node, named `<kind>:<path>`
/// (`tsproject:packages/lib/tsconfig.json`) and shared by all tasks that
/// contain the part.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPart {
    /// What the part is (`tsproject`); the prefix of its node name.
    pub kind: String,
    /// The file that identifies the part (absolute): the project's tsconfig.
    pub path: PathBuf,
    /// Config files (the tsconfig `extends` chain).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configs: Vec<PathBuf>,
    /// Files compiled into the part's outputs (tsconfig `files` and `include`
    /// entries that name a file, of a project that emits JavaScript).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PathBuf>,
    /// The same files where the part only reads them (a project that `tsc` only
    /// type-checks): see [`TaskSpec::data_inputs`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_inputs: Vec<PathBuf>,
    /// Directory memberships (tsconfig `include`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memberships: Vec<Membership>,
    /// Tool packages whose version is an input (`typescript`, config base
    /// packages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toolchain: Vec<Toolchain>,
    /// Parts (by [`TaskPart::path`]) this one builds on: referenced projects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<PathBuf>,
    /// Filesystem observations made while reading the part's configuration
    /// (probes of config locations, config reads), recorded like
    /// `Task::observations`.
    #[serde(skip)]
    pub observations: turborepo_trace_core::Observations,
}

/// A reference from one task to another, as written. [`crate::graph`] resolves
/// it to [`TaskId`]s against the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TaskRef {
    /// A task of the same package (`npm run lint`, turbo `"dependsOn":
    /// ["codegen"]`).
    Local {
        /// Task name.
        name: String,
    },
    /// The task in every workspace package this package depends on (turbo
    /// `^build`).
    Upstream {
        /// Task name.
        name: String,
    },
    /// A task of another package, by package name or repo-relative directory
    /// (`pnpm -F web run build`, turbo `web#build`).
    Package {
        /// Package name (`@acme/web`) or repo-relative directory (`apps/web`).
        package: String,
        /// Task name.
        name: String,
    },
    /// The task in every package that has it (`turbo run build` in a root
    /// script, `pnpm -r run build`).
    All {
        /// Task name.
        name: String,
    },
    /// An exact task id (make prerequisites, `make -C dir target`).
    Id {
        /// The id.
        id: TaskId,
    },
}

/// A task dependency: the referenced task(s) must run first, and their inputs
/// are this task's inputs too.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskDep {
    /// What is referenced.
    pub target: TaskRef,
    /// Why (`dependsOn`, `pre hook`, `npm run`, `run-s`).
    pub reason: String,
    /// The referenced script only formats files in place ([`crate::format`]): a
    /// task that produces outputs drops the dependency.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_place: bool,
}

/// Tests a task executes (the "runs" relation, see [`crate::runs`]). It is
/// separate from the task's inputs: a test does not depend on the job that runs
/// it for its *contents*, but a change to how the job is defined (runner image,
/// tool versions, environment, flags) can change the test's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TestRun {
    /// A test runner invocation: the test files of `framework` at or below
    /// `dir`.
    Tests {
        /// Framework id as test records use it (`jest`, `vitest`, `pytest`,
        /// `gotest`, `cargo-test`), or another runner's name
        /// (`playwright`, `mocha`, `node-test`).
        framework: String,
        /// Directory (absolute) the run is rooted at.
        dir: PathBuf,
    },
    /// Another task this one executes (`pnpm run test`, `turbo run test`, a
    /// reusable workflow's job): whatever that task runs.
    Task {
        /// The referenced task(s).
        target: TaskRef,
    },
    /// A command or script that starts tests in a way that can't be mapped:
    /// every test. Recorded only on evidence of a test launch (a task
    /// runner named together with a test task), never for a command that is
    /// merely not understood.
    Unknown {
        /// Why (the command, the script and the launch word found in it).
        reason: String,
    },
    /// A task runner started with a computed task name (`pnpm run "$TASK"`). It
    /// may be a test task or anything else: it counts as
    /// [`TestRun::Unknown`] only for a task that is otherwise known to run
    /// tests.
    Computed {
        /// The command.
        reason: String,
    },
    /// A service the task starts and leaves running for the commands after it
    /// (`docker compose up`). The tests the task runs can talk to it over
    /// the network, where no import shows: what the service is built from
    /// and configured with (the compose file, the build contexts of its
    /// services, bind-mounted files) is an input of those tests.
    Service {
        /// The task that defines the service (`docker:<compose file>`).
        target: TaskRef,
    },
}

/// A tool package a task runs: its version (from the lockfile) is an input
/// (`next@15.x`, `typescript`, `ghcr.io/org/base` for docker base images).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Toolchain {
    /// `npm`, `pypi`, `go`, `crates`, `docker`, `system`.
    pub ecosystem: String,
    /// Package name (`next`, `@prisma/client`, `node:20-alpine`).
    pub name: String,
}

impl Toolchain {
    /// An npm package.
    pub fn npm(name: impl Into<String>) -> Toolchain {
        Toolchain {
            ecosystem: "npm".into(),
            name: name.into(),
        }
    }
}

/// Something about a task that could not be determined statically. With the
/// default uncertainty policy an uncertain task is affected by any change in
/// its package.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskUncertainty {
    /// Category: usually [`UncertaintyKind::UnknownCommand`] or
    /// [`UncertaintyKind::ShellUnsupported`]; recognizers may use any core
    /// kind.
    #[serde(with = "kind_serde")]
    pub kind: UncertaintyKind,
    /// Detail: the unknown program, the unsupported construct, the unreadable
    /// config.
    pub detail: String,
}

impl TaskUncertainty {
    /// Creates a record.
    pub fn new(kind: UncertaintyKind, detail: impl Into<String>) -> TaskUncertainty {
        TaskUncertainty {
            kind,
            detail: detail.into(),
        }
    }
}

mod kind_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use turborepo_trace_core::UncertaintyKind;

    pub fn serialize<S: Serializer>(k: &UncertaintyKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(k.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<UncertaintyKind, D::Error> {
        let s = String::deserialize(d)?;
        crate::spec::parse_kind(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown kind {s}")))
    }
}

/// Every core uncertainty kind, for parsing the stable names back.
pub const UNCERTAINTY_KINDS: &[UncertaintyKind] = &[
    UncertaintyKind::ComputedSpecifier,
    UncertaintyKind::PartialSpecifier,
    UncertaintyKind::PatternTooBroad,
    UncertaintyKind::UnresolvedSpecifier,
    UncertaintyKind::AliasedLoader,
    UncertaintyKind::EvalLike,
    UncertaintyKind::EnvDependent,
    UncertaintyKind::CwdDependent,
    UncertaintyKind::ConditionalResolution,
    UncertaintyKind::GeneratedFile,
    UncertaintyKind::CaseMismatch,
    UncertaintyKind::ParseIncomplete,
    UncertaintyKind::UnsupportedSyntax,
    UncertaintyKind::OutsideBase,
    UncertaintyKind::UnknownCommand,
    UncertaintyKind::ShellUnsupported,
];

/// Parses a stable uncertainty kind name (`unknown_command`).
pub fn parse_kind(s: &str) -> Option<UncertaintyKind> {
    UNCERTAINTY_KINDS.iter().copied().find(|k| k.as_str() == s)
}

/// What a task depends on and produces. Filled by task sources
/// (declared parts) and recognizers (per command).
///
/// A task is affected when a change touches any of: [`inputs`](Self::inputs),
/// [`data_inputs`](Self::data_inputs), [`configs`](Self::configs), the traced
/// closures of [`entries`](Self::entries), files added to or removed from
/// [`memberships`](Self::memberships), the tasks in
/// [`task_deps`](Self::task_deps) (transitively), the lockfile resolution of
/// [`toolchain`](Self::toolchain) packages, or the producers of generated files
/// its entries import. [`uncertainties`](Self::uncertainties) are reported, and
/// with the default policy make the task affected by any change in its package.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Files read as data (redirections `< file`, `cp` sources,
    /// `schema.prisma`). A missing input is a probe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PathBuf>,
    /// Input files of which the task provably only reads the contents: it
    /// neither runs them nor writes code made from them (the files a `tsc`
    /// project lists that is only type-checked, turbo `inputs` that name a
    /// file). Like [`inputs`](Self::inputs) for changes; the graph store
    /// links them by data edges, so what such a file would load
    /// when it runs (its uncertainty) is not the task's or its consumers'
    /// concern. A file that another command of the task runs or copies is
    /// also an entry or a plain input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_inputs: Vec<PathBuf>,
    /// Files and directories written. Importers of an output that
    /// [links](Output::links_producer) depend on this task; in-place edits and
    /// CI side effects are recorded but do not link.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Output>,
    /// Packages written into the install location ([`OutputPackage`]).
    /// Importers of the package depend on this task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_packages: Vec<OutputPackage>,
    /// Tool configuration files (`tsconfig.json` chain, `vite.config.ts`,
    /// `turbo.json`). Config files written in code should also be listed in
    /// [`entries`](Self::entries) so that what they import counts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configs: Vec<PathBuf>,
    /// Files traced with the language plugins (script files, build entry
    /// points, code configs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<TaskEntry>,
    /// Directory memberships (tsconfig `include`, turbo `inputs`, a whole
    /// package dir for an unknown command).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memberships: Vec<Membership>,
    /// Tasks that run before this one and whose inputs count.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_deps: Vec<TaskDep>,
    /// Tool packages whose version is an input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toolchain: Vec<Toolchain>,
    /// Environment variable names (never values) the task depends on (turbo
    /// `env`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// What could not be determined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertainties: Vec<TaskUncertainty>,
    /// Tests the task executes ([`TestRun`]). Not an input: nothing flows from
    /// here into the task's own "affected" verdict.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<TestRun>,
    /// Files that *define how the task runs* (beyond the file that declares
    /// it): the script a command runs with an interpreter (`node
    /// ci/run.mjs`, `./ci/test.sh` and the files it sources), a test
    /// runner's config file, and for a CI job the files its steps name
    /// (`node-version-file: .nvmrc`, a local action's code). A subset of the
    /// entries, inputs and configs. A change to one of them changes how the
    /// task's tests run ([`TaskSpec::runs`]); a change to application code
    /// that such a script imports, or that a build or test command reads,
    /// does not: that is an input.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definition: Vec<PathBuf>,
    /// Absolute paths that a command of the task writes out
    /// ([`Command::writes`]), normalized, and that are not below the root
    /// of this checkout. An input, entry or config that is one of them is
    /// not looked up on the machine that builds the graph: it
    /// names the repository's files that its trailing components name
    /// ([`turborepo_trace_core::fs::seen_from_elsewhere`]).
    ///
    /// A path below the root of this checkout is not in the list: a runner
    /// writes its own location into the commands it is given
    /// (`{toxinidir}`, `{{justfile_directory()}}`), so such a path is
    /// looked up where it is. (A command that spells out the absolute
    /// path of one checkout is read that way at that one place too: there the
    /// task has the edge to the file, and not the probes of the path's
    /// other readings.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<PathBuf>,
    /// A long-running task (turbo `persistent: true`: a dev server, `tsc -w`).
    /// No task may depend on it and builds never run it, so its outputs
    /// become [`OutputKind::Persistent`] (recorded, not linked).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub persistent: bool,
    /// Self-contained units of the task ([`TaskPart`]): the tsconfig projects
    /// of a `tsc -b` build. Outputs tagged with a part ([`Output::part`])
    /// link importers to the part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<TaskPart>,
}

impl TaskSpec {
    /// Adds an input file.
    pub fn input(&mut self, p: impl Into<PathBuf>) {
        self.inputs.push(p.into());
    }

    /// Adds a config file.
    pub fn config(&mut self, p: impl Into<PathBuf>) {
        self.configs.push(p.into());
    }

    /// Adds an entry traced with `lang`.
    pub fn entry(&mut self, p: impl Into<PathBuf>, lang: EntryLang) {
        self.entries.push(TaskEntry {
            path: p.into(),
            lang,
        });
    }

    /// Adds a file or directory output ([`OutputKind::Artifact`]).
    pub fn output(&mut self, p: impl Into<PathBuf>) {
        self.outputs.push(Output::path(p));
    }

    /// Adds a file or directory output of kind `kind`.
    pub fn output_of_kind(&mut self, p: impl Into<PathBuf>, kind: OutputKind) {
        self.outputs.push(Output::of_kind(p, kind));
    }

    /// Adds a write by a generic command (`cp`/`mv` destination, `>`
    /// redirection, `touch`), classified by [`write_kind`]: an in-place
    /// edit of an existing source file, or an artifact.
    pub fn write(&mut self, p: impl Into<PathBuf>, repo_root: &Path) {
        let p = p.into();
        let kind = write_kind(&p, repo_root);
        self.outputs.push(Output::of_kind(p, kind));
    }

    /// Sets the kind of every output added since index `start` that is still
    /// the default [`OutputKind::Artifact`] (explicitly classified outputs
    /// keep their kind).
    pub fn classify_outputs_since(&mut self, start: usize, kind: OutputKind) {
        for o in self.outputs.iter_mut().skip(start) {
            if o.kind == OutputKind::Artifact {
                o.kind = kind;
            }
        }
    }

    /// Sets the kind of every output.
    pub fn classify_all_outputs(&mut self, kind: OutputKind) {
        for o in &mut self.outputs {
            o.kind = kind;
        }
    }

    /// Adds a package the task writes into the install location (resolved from
    /// the task's package root, see [`OutputPackage`]).
    pub fn output_package(&mut self, ecosystem: &str, name: impl Into<String>) {
        self.output_packages.push(OutputPackage {
            ecosystem: ecosystem.into(),
            name: name.into(),
            dir: None,
        });
    }

    /// Like [`TaskSpec::output_package`], resolved from `dir` instead of the
    /// package root.
    pub fn output_package_in(
        &mut self,
        ecosystem: &str,
        name: impl Into<String>,
        dir: impl Into<PathBuf>,
    ) {
        self.output_packages.push(OutputPackage {
            ecosystem: ecosystem.into(),
            name: name.into(),
            dir: Some(dir.into()),
        });
    }

    /// Adds a membership.
    pub fn membership(
        &mut self,
        dir: impl Into<PathBuf>,
        pattern: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.memberships.push(Membership::new(dir, pattern, reason));
    }

    /// Adds a task dependency.
    pub fn dep(&mut self, target: TaskRef, reason: impl Into<String>) {
        self.task_deps.push(TaskDep {
            target,
            reason: reason.into(),
            in_place: false,
        });
    }

    /// Adds a dependency on a script that only formats files in place
    /// ([`TaskDep::in_place`]).
    pub fn dep_in_place(&mut self, target: TaskRef, reason: impl Into<String>) {
        self.task_deps.push(TaskDep {
            target,
            reason: reason.into(),
            in_place: true,
        });
    }

    /// Adds a toolchain package.
    pub fn tool(&mut self, ecosystem: &str, name: impl Into<String>) {
        self.toolchain.push(Toolchain {
            ecosystem: ecosystem.into(),
            name: name.into(),
        });
    }

    /// Adds an uncertainty.
    pub fn uncertain(&mut self, kind: UncertaintyKind, detail: impl Into<String>) {
        self.uncertainties.push(TaskUncertainty::new(kind, detail));
    }

    /// Records that the task runs the tests of `framework` at or below `dir`.
    pub fn test_run(&mut self, framework: impl Into<String>, dir: impl Into<PathBuf>) {
        self.runs.push(TestRun::Tests {
            framework: framework.into(),
            dir: dir.into(),
        });
    }

    /// Records that the task executes the task(s) `target` (for the runs
    /// relation only).
    pub fn runs_task(&mut self, target: TaskRef) {
        self.runs.push(TestRun::Task { target });
    }

    /// Records that the task starts the service(s) `target` defines and leaves
    /// them running.
    pub fn starts_service(&mut self, target: TaskRef) {
        self.runs.push(TestRun::Service { target });
    }

    /// Records that the task starts tests that can't be mapped.
    pub fn runs_unknown(&mut self, reason: impl Into<String>) {
        self.runs.push(TestRun::Unknown {
            reason: reason.into(),
        });
    }

    /// Records that the task starts a task whose name is computed.
    pub fn runs_computed(&mut self, reason: impl Into<String>) {
        self.runs.push(TestRun::Computed {
            reason: reason.into(),
        });
    }

    /// Records a file that defines how the task runs
    /// ([`TaskSpec::definition`]).
    pub fn defines(&mut self, p: impl Into<PathBuf>) {
        self.definition.push(p.into());
    }

    /// Adds everything in `other` (a union; nothing is dropped).
    pub fn merge(&mut self, other: TaskSpec) {
        self.inputs.extend(other.inputs);
        self.data_inputs.extend(other.data_inputs);
        self.outputs.extend(other.outputs);
        self.output_packages.extend(other.output_packages);
        self.configs.extend(other.configs);
        self.entries.extend(other.entries);
        self.memberships.extend(other.memberships);
        self.task_deps.extend(other.task_deps);
        self.toolchain.extend(other.toolchain);
        self.env.extend(other.env);
        self.uncertainties.extend(other.uncertainties);
        self.runs.extend(other.runs);
        self.definition.extend(other.definition);
        self.persistent |= other.persistent;
        self.parts.extend(other.parts);
    }

    /// Drops every part: outputs tagged with one become outputs of the whole
    /// task. For a task that also runs commands whose writes are unknown
    /// (`tsc -b && node patch-dist.js`): what importers of its outputs read
    /// then depends on the whole task.
    pub fn drop_parts(&mut self) {
        self.parts.clear();
        for o in &mut self.outputs {
            o.part = None;
        }
    }

    /// Sorts and deduplicates every list (call once filling is done).
    pub fn normalize(&mut self) {
        fn sd<T: Ord>(v: &mut Vec<T>) {
            v.sort();
            v.dedup();
        }
        sd(&mut self.inputs);
        sd(&mut self.data_inputs);
        // The same output recorded with several kinds keeps the one that links most
        // (the union is never less sound): kinds sort from most to least linking.
        // (A part's output and the same path written by the task as a whole are both
        // kept.)
        self.outputs.sort_by(|a, b| {
            (&a.path, &a.pattern, &a.exclude, &a.part, a.kind)
                .cmp(&(&b.path, &b.pattern, &b.exclude, &b.part, b.kind))
        });
        self.outputs.dedup_by(|b, a| {
            a.path == b.path && a.pattern == b.pattern && a.exclude == b.exclude && a.part == b.part
        });
        // One part per path: the same project read twice gives the same part.
        self.parts
            .sort_by(|a, b| (&a.kind, &a.path).cmp(&(&b.kind, &b.path)));
        self.parts
            .dedup_by(|b, a| a.kind == b.kind && a.path == b.path);
        sd(&mut self.output_packages);
        sd(&mut self.configs);
        sd(&mut self.entries);
        sd(&mut self.memberships);
        sd(&mut self.task_deps);
        sd(&mut self.toolchain);
        sd(&mut self.env);
        sd(&mut self.uncertainties);
        sd(&mut self.runs);
        sd(&mut self.definition);
    }

    /// Whether nothing at all is recorded.
    pub fn is_empty(&self) -> bool {
        *self == TaskSpec::default()
    }
}

/// A shell redirection target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redirect {
    /// The operator: `<`, `>`, `>>`, `2>`, `2>>`, `&>`, `<<<`.
    pub op: String,
    /// The target word (expanded), relative to the command's cwd unless
    /// absolute.
    pub target: Word,
}

impl Redirect {
    /// Whether the redirection reads (`<`).
    pub fn is_input(&self) -> bool {
        self.op == "<"
    }

    /// Whether the target is `/dev/null` or a file descriptor (`2>&1`).
    pub fn is_null(&self) -> bool {
        self.target.text == "/dev/null" || self.target.text.starts_with('&')
    }
}

/// One shell word after quote removal and known-variable expansion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    /// The text.
    pub text: String,
    /// The word contained an unquoted glob character (`*`, `?`, `[`); `text`
    /// keeps it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub glob: bool,
    /// Part of the word could not be expanded (an unknown `$VAR`, `$(…)`);
    /// `text` keeps the unexpanded source for that part.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
}

impl Word {
    /// A plain literal word.
    pub fn lit(s: impl Into<String>) -> Word {
        Word {
            text: s.into(),
            glob: false,
            dynamic: false,
        }
    }

    /// Whether the word is fully known (not dynamic).
    pub fn is_static(&self) -> bool {
        !self.dynamic
    }
}

/// How a command is connected to the next one.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connector {
    /// End of the list (or `;` / newline).
    #[default]
    Seq,
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `|`.
    Pipe,
    /// `&` (background).
    Background,
}

/// One simple command of a shell line, as parsed by [`crate::shell`]. Never
/// executed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    /// Program and arguments (after `VAR=x` prefixes).
    pub argv: Vec<Word>,
    /// `VAR=x` assignments before the program (also bare assignments with no
    /// program).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assigns: Vec<(String, Word)>,
    /// Redirections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<Redirect>,
    /// The working directory (absolute), after any preceding `cd dir &&`.
    pub cwd: PathBuf,
    /// A `cd` to a computed directory came before: [`Command::cwd`] is not
    /// where the command runs, and its relative paths are not known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cwd_computed: bool,
    /// The connector to the next command.
    #[serde(default)]
    pub next: Connector,
    /// The source text of this command (for reports).
    pub text: String,
}

impl Command {
    /// The program name as written (`argv[0]`), if any.
    pub fn program(&self) -> Option<&str> {
        self.argv.first().map(|w| w.text.as_str())
    }

    /// The program's base name (`./node_modules/.bin/tsc` → `tsc`).
    pub fn program_name(&self) -> Option<&str> {
        self.program().map(|p| p.rsplit('/').next().unwrap_or(p))
    }

    /// The arguments after the program, as strings.
    pub fn args(&self) -> impl Iterator<Item = &str> {
        self.argv.iter().skip(1).map(|w| w.text.as_str())
    }

    /// The argument words after the program.
    pub fn arg_words(&self) -> &[Word] {
        self.argv.get(1..).unwrap_or(&[])
    }

    /// A copy with the first `n` words removed (for wrappers like `npx`,
    /// `cross-env`).
    pub fn shifted(&self, n: usize) -> Command {
        let mut c = self.clone();
        c.argv.drain(..n.min(c.argv.len()));
        c
    }

    /// Resolves `p` against the command's cwd. A relative path that leaves the
    /// repository and comes back in names what it names inside, wherever
    /// the checkout is ([`turborepo_trace_frameworks::fsx::join_written`]).
    pub fn path(&self, p: &str) -> PathBuf {
        turborepo_trace_frameworks::fsx::join_written(&self.cwd, p)
    }

    /// Whether the command's own text writes out the absolute path `p`: it is
    /// the code's (`cat /app/fixtures/x.json`), not one the tool made while
    /// it evaluated the command (`"$PWD/x"`, a path below a `cd` to an
    /// absolute directory), so where it lands on this machine says nothing
    /// about what it names ([`TaskSpec::written`]).
    pub fn writes(&self, p: &str) -> bool {
        p.starts_with('/') && self.text.contains(p)
    }
}

#[cfg(test)]
mod output_kind_tests {
    use super::*;

    #[test]
    fn links_producer_by_kind() {
        use OutputKind::*;
        let links: Vec<(OutputKind, bool)> = [
            Generated,
            Declared,
            Artifact,
            Persistent,
            CacheOnly,
            InPlace,
            CiSideEffect,
        ]
        .into_iter()
        .map(|k| (k, Output::of_kind("/r/x", k).links_producer()))
        .collect();
        assert_eq!(
            links,
            [
                (Generated, true),
                (Declared, true),
                (Artifact, true),
                (Persistent, false),
                (CacheOnly, false),
                (InPlace, false),
                (CiSideEffect, false),
            ]
        );
        assert_eq!(Output::path("/r/x").kind, Artifact);
    }

    #[test]
    fn write_kind_in_place_only_for_existing_source_files() {
        let root = std::env::temp_dir().join(format!("tt-write-kind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::write(root.join("src/a.ts"), "").unwrap();
        std::fs::write(root.join("dist/a.js"), "").unwrap();
        let k = |p: &str| write_kind(&root.join(p), &root);
        assert_eq!(k("package.json"), OutputKind::InPlace);
        assert_eq!(k("src/a.ts"), OutputKind::InPlace);
        // Missing, below a build dir, a directory, or outside the repo: an artifact.
        assert_eq!(k("package.json.tmp"), OutputKind::Artifact);
        assert_eq!(k("dist/a.js"), OutputKind::Artifact);
        assert_eq!(k("src"), OutputKind::Artifact);
        assert_eq!(
            write_kind(&root.join("package.json"), Path::new("/elsewhere")),
            OutputKind::Artifact
        );
        // The repo itself may live below a directory named like an output dir.
        let nested = root.join("build/repo");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("package.json"), "{}").unwrap();
        assert_eq!(
            write_kind(&nested.join("package.json"), &nested),
            OutputKind::InPlace
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_keeps_the_most_linking_kind() {
        let mut s = TaskSpec::default();
        s.output_of_kind("/r/a", OutputKind::InPlace);
        s.output_of_kind("/r/a", OutputKind::Generated);
        s.output_of_kind("/r/b", OutputKind::CiSideEffect);
        s.normalize();
        let kinds: Vec<(&str, OutputKind)> = s
            .outputs
            .iter()
            .map(|o| (o.path.to_str().unwrap(), o.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("/r/a", OutputKind::Generated),
                ("/r/b", OutputKind::CiSideEffect)
            ]
        );
    }

    #[test]
    fn classify_since_keeps_explicit_kinds() {
        let mut s = TaskSpec::default();
        s.output("/r/before");
        let start = s.outputs.len();
        s.output("/r/gen");
        s.output_of_kind("/r/fmt", OutputKind::InPlace);
        s.classify_outputs_since(start, OutputKind::Generated);
        let kinds: Vec<OutputKind> = s.outputs.iter().map(|o| o.kind).collect();
        assert_eq!(
            kinds,
            [
                OutputKind::Artifact,
                OutputKind::Generated,
                OutputKind::InPlace
            ]
        );
    }
}
