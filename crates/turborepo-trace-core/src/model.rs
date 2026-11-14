//! The core data model: ids, reference kinds, specifiers, raw
//! references, edges, uncertainty records and inclusion reasons.

use std::{borrow::Cow, fmt};

use bitflags::bitflags;

pub use crate::lexkit::Span;

/// A dense id for an interned absolute path. See [`crate::PathInterner`].
///
/// Ids are only meaningful together with the interner that produced them. Their
/// numeric order depends on discovery order, which varies between runs and
/// thread counts, so never sort output by `FileId`; sort by path instead.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub(crate) u32);

impl FileId {
    /// The raw index into the interner.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// The raw `u32` value.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// What a reference means to the program that contains it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RefKind {
    /// Static import (`import x from`, `import a.b`, Go `import`).
    Import,
    /// CommonJS-style `require()`.
    Require,
    /// `import()` and other lazily evaluated module loads.
    DynamicImport,
    /// `export ... from`.
    ReExport,
    /// A Go package: fans out to the package's files.
    Package,
    /// Rust `mod x;`.
    Mod,
    /// Compile-time embedding: `//go:embed`, `include_str!`, `include_dir!`.
    Embed,
    /// Runtime file read (`fs.readFileSync`, `open()`, `std::fs::read`).
    FsRead,
    /// Runtime directory read (`fs.readdirSync`, `os.listdir`).
    FsReadDir,
    /// A worker script (`new Worker(...)`).
    Worker,
    /// A child process script (`child_process.fork`).
    ChildProcess,
    /// A static asset referenced by location rather than read through a tracked
    /// API, such as `new URL('./x.png', import.meta.url)` or a data file
    /// located by a package special case. A leaf with reason `asset`.
    Asset,
    /// A native addon (`.node`, Python extension module).
    NativeAddon,
    /// A shared library loaded by a native addon.
    SharedLib,
    /// A manifest consulted during resolution (`package.json`, `go.mod`,
    /// `Cargo.toml`).
    Manifest,
}

impl RefKind {
    /// Every kind, in declaration order.
    pub const ALL: [RefKind; 15] = [
        RefKind::Import,
        RefKind::Require,
        RefKind::DynamicImport,
        RefKind::ReExport,
        RefKind::Package,
        RefKind::Mod,
        RefKind::Embed,
        RefKind::FsRead,
        RefKind::FsReadDir,
        RefKind::Worker,
        RefKind::ChildProcess,
        RefKind::NativeAddon,
        RefKind::SharedLib,
        RefKind::Manifest,
        RefKind::Asset,
    ];

    /// Stable snake_case name used in outputs.
    pub const fn as_str(self) -> &'static str {
        match self {
            RefKind::Import => "import",
            RefKind::Require => "require",
            RefKind::DynamicImport => "dynamic_import",
            RefKind::ReExport => "re_export",
            RefKind::Package => "package",
            RefKind::Mod => "mod",
            RefKind::Embed => "embed",
            RefKind::FsRead => "fs_read",
            RefKind::FsReadDir => "fs_read_dir",
            RefKind::Worker => "worker",
            RefKind::ChildProcess => "child_process",
            RefKind::NativeAddon => "native_addon",
            RefKind::SharedLib => "shared_lib",
            RefKind::Manifest => "manifest",
            RefKind::Asset => "asset",
        }
    }

    /// Whether targets of this kind are source code that should itself be
    /// extracted.
    ///
    /// Assets (reads, embeds), native binaries and manifests are leaves: they
    /// are emitted but never parsed, even if their extension matches a
    /// language.
    pub const fn follows(self) -> bool {
        matches!(
            self,
            RefKind::Import
                | RefKind::Require
                | RefKind::DynamicImport
                | RefKind::ReExport
                | RefKind::Package
                | RefKind::Mod
                | RefKind::Worker
                | RefKind::ChildProcess
        )
    }

    /// The phase a reference of this kind belongs to unless the language says
    /// otherwise.
    pub const fn default_phase(self) -> Phase {
        match self {
            RefKind::Embed | RefKind::Mod | RefKind::Package => Phase::Build,
            _ => Phase::Runtime,
        }
    }

    /// The nft-style inclusion reason for a target reached through this kind.
    ///
    /// Native addons count as assets, as in nft, where binaries located by
    /// `bindings`, `node-gyp-build` or `node-pre-gyp` are emitted as
    /// assets.
    pub const fn reason(self) -> ReasonKinds {
        match self {
            RefKind::Embed
            | RefKind::FsRead
            | RefKind::FsReadDir
            | RefKind::Asset
            | RefKind::NativeAddon => ReasonKinds::ASSET,
            RefKind::SharedLib => ReasonKinds::SHAREDLIB,
            RefKind::Manifest => ReasonKinds::MANIFEST,
            _ => ReasonKinds::DEPENDENCY,
        }
    }
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When a file is needed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    /// Needed to run the program.
    Runtime,
    /// Needed only to compile it (Go/Rust sources, embeds).
    Build,
}

impl Phase {
    /// Stable lowercase name used in outputs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Runtime => "runtime",
            Phase::Build => "build",
        }
    }
}

/// Which phases a trace includes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PhaseFilter {
    /// Runtime files only.
    Runtime,
    /// Build files only (entries are always included).
    Build,
    /// Everything (default).
    #[default]
    All,
}

impl PhaseFilter {
    /// Whether an edge in `phase` passes this filter.
    pub const fn allows(self, phase: Phase) -> bool {
        match self {
            PhaseFilter::All => true,
            PhaseFilter::Runtime => matches!(phase, Phase::Runtime),
            PhaseFilter::Build => matches!(phase, Phase::Build),
        }
    }
}

bitflags! {
    /// Qualifiers on a reference.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct RefFlags: u16 {
        /// Inside `try`/`catch`, `try`/`except ImportError`, ...: failure is tolerated.
        const OPTIONAL    = 1 << 0;
        /// Guarded by cfg, build tags, `sys.platform`, a ternary branch, ...
        const CONDITIONAL = 1 << 1;
        /// Dynamic import, function-local import.
        const LAZY        = 1 << 2;
        /// `import type`, `if TYPE_CHECKING:`. Excluded from output unless types are included.
        const TYPE_ONLY   = 1 << 3;
        /// The source reads or compiles the target as an input; it does not execute it. Set
        /// on a build task's edges to its input files and membership hubs. What the target
        /// would load when run (its uncertainty) is not the source's concern; a change of the
        /// target still is.
        const DATA        = 1 << 4;
        /// The reference observes names only: whether a path exists, what kind of entry
        /// it is, or which entries a directory has (`existsSync`, `access`, `readdir`, a
        /// `stat` used for `isDirectory()`). No edit of a file's contents can change what
        /// it sees; only adding or removing a path can, and the directory memberships
        /// recorded for the reference cover that. Set by an extractor that knows the API.
        const NAMES       = 1 << 5;
        /// The reference locates a path without a tracked call reading it:
        /// `new URL('./gen.mjs', import.meta.url)`. What is done with the path is not
        /// known (it may be handed to a process or a bundler), so the graph store does not
        /// treat the edge as a read of the target's bytes. Not a named flag: outputs don't
        /// show it.
        const LOCATED     = 1 << 6;
        /// The target is the configuration of the tool that runs the source (a test's runner
        /// config): the tool loads it, not the source. The source depends on it (changes and
        /// uncertainty propagate), but its code does not run with the source's code, so the
        /// strings of one are not path names for the reads of the other
        /// (`turborepo_trace_graph::names`).
        const CONFIG      = 1 << 7;
    }
}

impl RefFlags {
    /// Lowercase names of the set flags, in bit order, for outputs.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        [
            (RefFlags::OPTIONAL, "optional"),
            (RefFlags::CONDITIONAL, "conditional"),
            (RefFlags::LAZY, "lazy"),
            (RefFlags::TYPE_ONLY, "type_only"),
            (RefFlags::DATA, "data"),
            (RefFlags::NAMES, "names"),
            (RefFlags::CONFIG, "config"),
        ]
        .into_iter()
        .filter(move |(f, _)| self.contains(*f))
        .map(|(_, n)| n)
    }
}

/// One piece of a partially known specifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Part<'src> {
    /// Known text.
    Lit(Cow<'src, str>),
    /// Unknown text. `reason` records the source expression or why it is
    /// unknown.
    Wild {
        /// Source text of the unknown expression, or a short explanation.
        reason: Cow<'src, str>,
    },
}

impl Part<'_> {
    /// Converts to an owned part with a `'static` lifetime.
    pub fn into_owned(self) -> Part<'static> {
        match self {
            Part::Lit(s) => Part::Lit(Cow::Owned(s.into_owned())),
            Part::Wild { reason } => Part::Wild {
                reason: Cow::Owned(reason.into_owned()),
            },
        }
    }
}

/// What a reference names, as far as static analysis can tell.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Specifier<'src> {
    /// Fully known, for example `'./a.js'`.
    Literal(Cow<'src, str>),
    /// Partially known, for example `["./locales/", Wild, ".json"]`.
    ///
    /// A `Vec` rather than a `SmallVec<[Part; 4]>`: `SmallVec` is
    /// invariant in its item type, which would make `Specifier<'src>`
    /// invariant and stop a `Specifier<'static>` from being used where a
    /// `Specifier<'src>` is expected. Patterns only occur for partially
    /// known specifiers, so the allocation is off the hot path.
    Pattern(Vec<Part<'src>>),
    /// One of several values, for example from a ternary. Capped by the
    /// evaluator.
    ///
    /// A `Vec` rather than an inline `SmallVec<[Specifier; 2]>`,
    /// which would be an infinitely sized recursive type.
    Alternatives(Vec<Specifier<'src>>),
    /// Nothing is known.
    Unknown,
}

impl<'src> Specifier<'src> {
    /// A borrowed literal specifier.
    pub fn literal(s: impl Into<Cow<'src, str>>) -> Self {
        Specifier::Literal(s.into())
    }

    /// The literal text, if fully known.
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Specifier::Literal(s) => Some(s),
            _ => None,
        }
    }

    /// Converts to an owned specifier with a `'static` lifetime.
    pub fn into_owned(self) -> Specifier<'static> {
        match self {
            Specifier::Literal(s) => Specifier::Literal(Cow::Owned(s.into_owned())),
            Specifier::Pattern(parts) => {
                Specifier::Pattern(parts.into_iter().map(Part::into_owned).collect())
            }
            Specifier::Alternatives(alts) => {
                Specifier::Alternatives(alts.into_iter().map(Specifier::into_owned).collect())
            }
            Specifier::Unknown => Specifier::Unknown,
        }
    }
}

/// Displays literals as-is, patterns with `*` for each wildcard, alternatives
/// joined by ` | `, and unknown as `<unknown>`.
impl fmt::Display for Specifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Specifier::Literal(s) => f.write_str(s),
            Specifier::Pattern(parts) => {
                for p in parts {
                    match p {
                        Part::Lit(s) => f.write_str(s)?,
                        Part::Wild { .. } => f.write_str("*")?,
                    }
                }
                Ok(())
            }
            Specifier::Alternatives(alts) => {
                for (i, a) in alts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{a}")?;
                }
                Ok(())
            }
            Specifier::Unknown => f.write_str("<unknown>"),
        }
    }
}

/// The directory a relative specifier is anchored to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    /// The directory of the referencing file (`./x` imports, `__dirname`).
    FileDir,
    /// The process working directory at runtime (`--cwd`).
    Cwd,
    /// The trace base / project root (`--base`).
    ProjectRoot,
    /// A specific directory.
    Dir(FileId),
    /// A language-defined anchor such as `"crate_root"`, interpreted by the
    /// language's resolver.
    Language(&'static str),
}

/// A reference found by a language extractor. Borrows from the source buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRef<'src> {
    /// What kind of reference this is.
    pub kind: RefKind,
    /// What it names.
    pub spec: Specifier<'src>,
    /// What a relative specifier is anchored to.
    pub base: Base,
    /// Where it occurs in the source.
    pub span: Span,
    /// Qualifiers.
    pub flags: RefFlags,
    /// When the target is needed. Defaults to [`RefKind::default_phase`].
    pub phase: Phase,
    /// Set when the extractor already knows the reference is uncertain
    /// (computed, partial, aliased loader, ...).
    pub uncertainty: Option<Uncertainty>,
}

impl<'src> RawRef<'src> {
    /// A reference anchored at the file's directory with no flags and the
    /// kind's default phase.
    pub fn new(kind: RefKind, spec: Specifier<'src>, span: Span) -> Self {
        RawRef {
            kind,
            spec,
            base: Base::FileDir,
            span,
            flags: RefFlags::empty(),
            phase: kind.default_phase(),
            uncertainty: None,
        }
    }

    /// Sets the base.
    pub fn with_base(mut self, base: Base) -> Self {
        self.base = base;
        self
    }

    /// Adds flags.
    pub fn with_flags(mut self, flags: RefFlags) -> Self {
        self.flags |= flags;
        self
    }

    /// Sets the phase.
    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    /// Attaches an uncertainty record.
    pub fn with_uncertainty(mut self, u: Uncertainty) -> Self {
        self.uncertainty = Some(u);
        self
    }
}

/// Where an edge points.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A single file.
    File(FileId),
    /// A directory; every file below it is included (capped by the glob limit).
    Dir(FileId),
    /// A glob expanded against the filesystem.
    Glob {
        /// The glob as written, with `*` for each wildcard.
        pattern: Box<str>,
        /// Matching files.
        matches: Vec<FileId>,
        /// Whether matching stopped at the match limit.
        truncated: bool,
    },
    /// A builtin module (`node:fs`, Python stdlib, Go std).
    Builtin(Box<str>),
    /// A third-party package that is not traversed because of `--externals
    /// stop|list`: its name, such as `react` or `@scope/pkg`.
    External(Box<str>),
    /// Resolution failed. The edge carries an uncertainty.
    Unresolved,
}

/// A resolved reference from one file to its target(s).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    /// The referencing file.
    pub from: FileId,
    /// What it points to.
    pub to: Target,
    /// Reference kind.
    pub kind: RefKind,
    /// When the target is needed.
    pub phase: Phase,
    /// Qualifiers.
    pub flags: RefFlags,
    /// Source location in `from`.
    pub span: Span,
    /// The specifier as written (display form), for reports.
    pub spec: Box<str>,
    /// Why this edge is not certain, if it is not.
    pub uncertainty: Option<Uncertainty>,
}

/// A record of something static analysis could not fully determine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Uncertainty {
    /// Category.
    pub kind: UncertaintyKind,
    /// Source location.
    pub span: Span,
    /// Human-readable detail: the expression, the narrowed glob, the failing
    /// specifier, ...
    pub detail: Box<str>,
    /// The known bound of the reference's unknown target set, if there is one:
    /// a *covered* uncertainty. `None` is an *open* uncertainty (nothing
    /// bounds it: `eval`, a fully computed path, an unknown working
    /// directory).
    ///
    /// Contract: every file the reference can read, now or after any change,
    /// lies inside the scope, and the reference only reads those files'
    /// contents (it does not execute them as modules, so their own
    /// dependencies don't matter). A change query then seeds
    /// the referencing file on any added, modified or deleted path inside the
    /// scope, and the uncertainty doesn't make the selection uncertain.
    pub scope: Option<UncertaintyScope>,
    /// Where the unknown part of the reference's value comes from, as far as
    /// the analysis can tell ([`Origin::Unknown`] when it can't).
    pub origin: Origin,
}

/// Where the unknown part of an uncertain reference's value comes from. A
/// change query treats the classes differently: a path built from a function
/// parameter is the responsibility of the callers that construct it, a path
/// from the environment is an input from outside the repository, and so on.
/// Values the analysis knows to lie outside the repository (temporary
/// directories, absolute system paths) are not uncertain at all and get no
/// record.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Not analyzed, or the analysis could not tell.
    #[default]
    Unknown,
    /// A parameter of the enclosing function, bound by callers the file's
    /// analysis can't see (an exported function, a callback, a method).
    Param,
    /// The environment (`process.env.X`, `os.environ`).
    Env,
    /// The process working directory, with no known path below it.
    Cwd,
    /// The command line (`process.argv`, `sys.argv`).
    Argv,
    /// A binding imported from another module whose value this file's analysis
    /// can't see.
    Imported,
    /// The result of a call the analysis can't evaluate.
    Call,
    /// Data only known at runtime: parsed files, network responses, database
    /// rows, input.
    Data,
}

impl Origin {
    /// Every origin, in declaration order (the index stored in shards).
    pub const ALL: [Origin; 8] = [
        Origin::Unknown,
        Origin::Param,
        Origin::Env,
        Origin::Cwd,
        Origin::Argv,
        Origin::Imported,
        Origin::Call,
        Origin::Data,
    ];

    /// Stable snake_case name used in outputs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Origin::Unknown => "unknown",
            Origin::Param => "param",
            Origin::Env => "env",
            Origin::Cwd => "cwd",
            Origin::Argv => "argv",
            Origin::Imported => "imported",
            Origin::Call => "call",
            Origin::Data => "data",
        }
    }
}

/// What bounds a covered [`Uncertainty`]: every file below `dir` whose path
/// relative to `dir` matches `pattern`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UncertaintyScope {
    /// Absolute, normalized directory.
    pub dir: std::path::PathBuf,
    /// Glob relative to `dir` (`*`, `?`, `[..]`, `{a,b}`, `**` segments; `*`
    /// never matches `/`). `**` is every file below `dir`.
    pub pattern: Box<str>,
}

impl UncertaintyScope {
    /// Every file below `dir`.
    pub fn below(dir: impl Into<std::path::PathBuf>) -> Self {
        UncertaintyScope {
            dir: dir.into(),
            pattern: "**".into(),
        }
    }
}

impl Uncertainty {
    /// Creates an open uncertainty record.
    pub fn new(kind: UncertaintyKind, span: Span, detail: impl Into<Box<str>>) -> Self {
        Uncertainty {
            kind,
            span,
            detail: detail.into(),
            scope: None,
            origin: Origin::Unknown,
        }
    }

    /// Sets the origin of the unknown value (see [`Uncertainty::origin`]).
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = origin;
        self
    }

    /// Sets the scope (see [`Uncertainty::scope`]), making the record covered.
    pub fn with_scope(mut self, scope: UncertaintyScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Whether the record has a scope.
    pub fn is_covered(&self) -> bool {
        self.scope.is_some()
    }
}

/// Categories of uncertainty.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UncertaintyKind {
    /// `require(x)`: nothing known.
    ComputedSpecifier,
    /// `require('./a/' + x)`: narrowed to a glob.
    PartialSpecifier,
    /// Glob over the match limit, truncated.
    PatternTooBroad,
    /// Literal, but resolution failed.
    UnresolvedSpecifier,
    /// `const r = require`, shadowed `require`, importlib alias.
    AliasedLoader,
    /// `eval`, `new Function`, `exec`, proc macros.
    EvalLike,
    /// `process.env.X`, `os.environ`, `env!("...")`.
    EnvDependent,
    /// A relative fs path not anchored to the file.
    CwdDependent,
    /// Exports conditions, build tags, cfg, `sys.platform`.
    ConditionalResolution,
    /// `OUT_DIR`, build.rs output, codegen.
    GeneratedFile,
    /// Resolves only case-insensitively (works on macOS, breaks on Linux).
    CaseMismatch,
    /// Lexer error recovery, or the file could not be read.
    ParseIncomplete,
    /// A known construct deliberately not modelled.
    UnsupportedSyntax,
    /// A file resolved outside the trace base (`--base`), for example an import
    /// that walked up to a `node_modules` above the project. Such files are
    /// omitted from `nft` and `list` output, so the deployment may miss
    /// them. Reported once per outside file, on the file that first
    /// references it.
    OutsideBase,
    /// A build-task command whose program no recognizer knows. The
    /// task depends on its whole package directory; the detail names the
    /// program.
    UnknownCommand,
    /// Shell syntax in a task command that is deliberately not modelled:
    /// command substitution, loops, conditionals, functions, `eval`,
    /// unknown variables.
    ShellUnsupported,
}

impl UncertaintyKind {
    /// Stable snake_case name used in outputs.
    pub const fn as_str(self) -> &'static str {
        match self {
            UncertaintyKind::ComputedSpecifier => "computed_specifier",
            UncertaintyKind::PartialSpecifier => "partial_specifier",
            UncertaintyKind::PatternTooBroad => "pattern_too_broad",
            UncertaintyKind::UnresolvedSpecifier => "unresolved_specifier",
            UncertaintyKind::AliasedLoader => "aliased_loader",
            UncertaintyKind::EvalLike => "eval_like",
            UncertaintyKind::EnvDependent => "env_dependent",
            UncertaintyKind::CwdDependent => "cwd_dependent",
            UncertaintyKind::ConditionalResolution => "conditional_resolution",
            UncertaintyKind::GeneratedFile => "generated_file",
            UncertaintyKind::CaseMismatch => "case_mismatch",
            UncertaintyKind::ParseIncomplete => "parse_incomplete",
            UncertaintyKind::UnsupportedSyntax => "unsupported_syntax",
            UncertaintyKind::OutsideBase => "outside_base",
            UncertaintyKind::UnknownCommand => "unknown_command",
            UncertaintyKind::ShellUnsupported => "shell_unsupported",
        }
    }
}

impl fmt::Display for UncertaintyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

bitflags! {
    /// Why a file is part of the output, in the style of nft's `reasons[].type`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct ReasonKinds: u8 {
        /// An entry point.
        const INITIAL    = 1 << 0;
        /// Encountered during resolution, such as a symlink target.
        const RESOLVE    = 1 << 1;
        /// A module dependency.
        const DEPENDENCY = 1 << 2;
        /// An asset read, embedded or globbed.
        const ASSET      = 1 << 3;
        /// A shared library.
        const SHAREDLIB  = 1 << 4;
        /// A manifest consulted during resolution.
        const MANIFEST   = 1 << 5;
    }
}

impl ReasonKinds {
    /// Lowercase names of the set reasons, in bit order, for outputs.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        [
            (ReasonKinds::INITIAL, "initial"),
            (ReasonKinds::RESOLVE, "resolve"),
            (ReasonKinds::DEPENDENCY, "dependency"),
            (ReasonKinds::ASSET, "asset"),
            (ReasonKinds::SHAREDLIB, "sharedlib"),
            (ReasonKinds::MANIFEST, "manifest"),
        ]
        .into_iter()
        .filter(move |(f, _)| self.contains(*f))
        .map(|(_, n)| n)
    }
}

/// Why one file is included: reason kinds plus the files that pulled it in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileReason {
    /// Union of reason kinds over all incoming edges.
    pub kinds: ReasonKinds,
    /// Files with an edge to this one, sorted by path, without duplicates.
    pub parents: Vec<FileId>,
}

/// Trace-level uncertainty that is attached to a file but not to an edge, such
/// as a lexer error or a read failure.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileDiagnostic {
    /// The affected file.
    pub file: FileId,
    /// What happened.
    pub uncertainty: Uncertainty,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specifier_display() {
        let lit = Specifier::literal("./a.js");
        assert_eq!(lit.to_string(), "./a.js");
        let pat = Specifier::Pattern(vec![
            Part::Lit("./locales/".into()),
            Part::Wild {
                reason: "lang".into(),
            },
            Part::Lit(".json".into()),
        ]);
        assert_eq!(pat.to_string(), "./locales/*.json");
        let alts = Specifier::Alternatives(vec![lit.clone(), Specifier::Unknown]);
        assert_eq!(alts.to_string(), "./a.js | <unknown>");
        assert_eq!(alts.into_owned().to_string(), "./a.js | <unknown>");
    }

    #[test]
    fn kinds_and_flags() {
        assert!(RefKind::Import.follows());
        assert!(!RefKind::FsRead.follows());
        assert_eq!(RefKind::Embed.default_phase(), Phase::Build);
        assert_eq!(RefKind::ALL.len(), 15);
        assert!(!RefKind::Asset.follows());
        assert_eq!(RefKind::Asset.reason(), ReasonKinds::ASSET);
        assert_eq!(RefKind::SharedLib.reason(), ReasonKinds::SHAREDLIB);
        let names: Vec<_> = (RefFlags::OPTIONAL | RefFlags::TYPE_ONLY).names().collect();
        assert_eq!(names, ["optional", "type_only"]);
        let names: Vec<_> = (ReasonKinds::INITIAL | ReasonKinds::ASSET)
            .names()
            .collect();
        assert_eq!(names, ["initial", "asset"]);
        assert!(PhaseFilter::Runtime.allows(Phase::Runtime));
        assert!(!PhaseFilter::Runtime.allows(Phase::Build));
    }

    #[test]
    fn raw_ref_builder() {
        let r = RawRef::new(RefKind::FsRead, Specifier::Unknown, Span::new(1, 2))
            .with_base(Base::Cwd)
            .with_flags(RefFlags::OPTIONAL)
            .with_phase(Phase::Build);
        assert_eq!(r.base, Base::Cwd);
        assert_eq!(r.flags, RefFlags::OPTIONAL);
        assert_eq!(r.phase, Phase::Build);
    }
}
