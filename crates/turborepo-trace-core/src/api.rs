//! Tracked runtime APIs as data.
//!
//! Every language keeps a `const` table of [`ApiEntry`] values in this shape:
//! calling export `path` of one of `modules` (or the global `path` when
//! `modules` is empty) in call style `style` with argument number `arg`
//! references a file of kind `kind`, needed in `phase`, anchored by `base`,
//! with extra `flags`. Examples:
//!
//! | Language | Entry |
//! |---|---|
//! | JS | `fs.readFileSync(p)`: `ApiEntry::new(&["fs"], "readFileSync", RefKind::FsRead)` |
//! | JS | `new Worker(p)`: `ApiEntry::new(&["worker_threads"], "Worker", RefKind::Worker).style(CallStyle::New)` |
//! | Python | `importlib.import_module(m)`: `ApiEntry::new(&["importlib"], "import_module", RefKind::DynamicImport).base(BaseRule::Module)` |
//! | Python | `pkgutil.get_data(pkg, res)`: `ApiEntry::new(&["pkgutil"], "get_data", RefKind::FsRead).arg(1).base(BaseRule::PackageRoot)` |
//! | Go | `os.ReadFile(p)`: `ApiEntry::new(&["os"], "ReadFile", RefKind::FsRead)` |
//! | Rust | `include_str!(p)`: `ApiEntry::new(&[], "include_str", RefKind::Embed).style(CallStyle::Macro).base(BaseRule::FileDir)` |
//! | Rust | `#[folder = "p"]` of rust-embed: `ApiEntry::new(&["rust_embed"], "folder", RefKind::Embed).style(CallStyle::Attribute).base(BaseRule::PackageRoot)` |
//! | JS | pino transport `{ target: 'm' }`: `ApiEntry::new(&["pino"], "target", RefKind::Require).style(CallStyle::Property).role(ArgRole::ModuleString)` |
//! | Python | `importlib.import_module(m)` as a loader whose wrappers are tracked: `.role(ArgRole::LoaderWrapper)` |
//!
//! Matching call sites to entries (import bindings, receivers, aliases) is
//! language-specific; [`lookup`] and [`table_has_module`] are the shared exact
//! lookups. User entries from a configuration file could be appended in the
//! same shape.

use crate::{Phase, RefFlags, RefKind};

/// How a path argument is anchored.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BaseRule {
    /// A file system path: relative strings are relative to the process working
    /// directory (`Base::Cwd` with `CwdDependent`); paths built from file
    /// anchors (`__dirname`, `__file__`) are relative to the file's
    /// directory.
    Path,
    /// A module specifier: relative strings are relative to the file's
    /// directory, bare names are packages.
    Module,
    /// Always relative to the referencing file's directory (`include_str!`,
    /// `//go:embed`).
    FileDir,
    /// Relative to the root of the package, crate or module containing the file
    /// (Cargo.toml directory for rust-embed, the named package for
    /// `pkgutil.get_data`); the language resolver interprets it (usually
    /// through `Base::Language`).
    PackageRoot,
}

/// How the API is invoked.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallStyle {
    /// `f(x)`.
    Call,
    /// `new F(x)`.
    New,
    /// A macro invocation, `name!(x)`.
    Macro,
    /// An attribute or decorator argument, `#[name = x]`; `path` is the
    /// attribute key.
    Attribute,
    /// A property or assignment whose value is the argument: `{ key: x }` in an
    /// object or dict literal, or `KEY = x` at module level; `path` is the
    /// key (pino `target`, Django `INSTALLED_APPS`). With empty `modules`
    /// it applies to every file, otherwise only to files importing one of
    /// them.
    Property,
}

/// What the tracked argument is.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArgRole {
    /// A path or module specifier, interpreted per `kind` and `base` (the
    /// default).
    #[default]
    Path,
    /// A string naming a module that a library loads later (pino transport
    /// `target`, Django `INSTALLED_APPS`, entry points). It becomes an
    /// `OPTIONAL` module reference of `kind` with an `AliasedLoader`
    /// uncertainty: the name is only a guess at what gets loaded.
    ModuleString,
    /// A loader (`require`, `importlib.import_module`, `plugin.Open`,
    /// `Library::new`) whose wrappers are tracked: a function passing one
    /// of its parameters to it makes each call of that function a reference
    /// of the same `kind`, with an `AliasedLoader` uncertainty.
    LoaderWrapper,
}

/// One tracked API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiEntry {
    /// Module specifiers (normalized by the language, e.g. without `node:`), or
    /// empty for a global or builtin.
    pub modules: &'static [&'static str],
    /// The dotted export path inside the module (`readFileSync`,
    /// `promises.readFile`, `Path.read_text`).
    pub path: &'static str,
    /// The index of the path argument.
    pub arg: u8,
    /// The reference kind.
    pub kind: RefKind,
    /// When the target is needed.
    pub phase: Phase,
    /// How the path is anchored.
    pub base: BaseRule,
    /// Extra flags (`OPTIONAL` for existence probes).
    pub flags: RefFlags,
    /// How the API is invoked.
    pub style: CallStyle,
    /// What the argument is.
    pub role: ArgRole,
}

impl ApiEntry {
    /// An entry with argument 0, the kind's default phase, [`BaseRule::Path`],
    /// no flags and [`CallStyle::Call`].
    pub const fn new(modules: &'static [&'static str], path: &'static str, kind: RefKind) -> Self {
        ApiEntry {
            modules,
            path,
            arg: 0,
            kind,
            phase: kind.default_phase(),
            base: BaseRule::Path,
            flags: RefFlags::empty(),
            style: CallStyle::Call,
            role: ArgRole::Path,
        }
    }

    /// Sets the argument index.
    pub const fn arg(mut self, arg: u8) -> Self {
        self.arg = arg;
        self
    }

    /// Sets the phase.
    pub const fn phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    /// Sets the base rule.
    pub const fn base(mut self, base: BaseRule) -> Self {
        self.base = base;
        self
    }

    /// Sets the flags.
    pub const fn flags(mut self, flags: RefFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Sets the call style.
    pub const fn style(mut self, style: CallStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the argument role.
    pub const fn role(mut self, role: ArgRole) -> Self {
        self.role = role;
        self
    }

    /// Whether this entry is for `path` of `module` (`None` = a global) in
    /// `style`.
    #[inline]
    pub fn matches(&self, module: Option<&str>, path: &str, style: CallStyle) -> bool {
        self.path == path
            && self.style == style
            && match module {
                None => self.modules.is_empty(),
                Some(m) => self.modules.contains(&m),
            }
    }
}

/// Finds the entry of `table` for `path` exported by `module` (`None` = a
/// global).
#[inline]
pub fn lookup<'t>(
    table: &'t [ApiEntry],
    module: Option<&str>,
    path: &str,
    style: CallStyle,
) -> Option<&'t ApiEntry> {
    table.iter().find(|e| e.matches(module, path, style))
}

/// The entries of `table` with role `role`.
pub fn with_role(table: &[ApiEntry], role: ArgRole) -> impl Iterator<Item = &ApiEntry> {
    table.iter().filter(move |e| e.role == role)
}

/// Whether any entry of `table` belongs to `module`.
#[inline]
pub fn table_has_module(table: &[ApiEntry], module: &str) -> bool {
    table.iter().any(|e| e.modules.contains(&module))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &[ApiEntry] = &[
        ApiEntry::new(&["fs"], "readFileSync", RefKind::FsRead),
        ApiEntry::new(&["fs"], "existsSync", RefKind::FsRead).flags(RefFlags::OPTIONAL),
        ApiEntry::new(&[], "Worker", RefKind::Worker).style(CallStyle::New),
        ApiEntry::new(&[], "include_str", RefKind::Embed)
            .style(CallStyle::Macro)
            .base(BaseRule::FileDir),
    ];

    #[test]
    fn lookups() {
        let e = lookup(T, Some("fs"), "readFileSync", CallStyle::Call).unwrap();
        assert_eq!(
            (e.kind, e.phase, e.arg),
            (RefKind::FsRead, Phase::Runtime, 0)
        );
        assert!(
            lookup(T, Some("fs"), "existsSync", CallStyle::Call)
                .unwrap()
                .flags
                .contains(RefFlags::OPTIONAL)
        );
        assert!(lookup(T, Some("fs"), "readFileSync", CallStyle::New).is_none());
        assert!(lookup(T, None, "Worker", CallStyle::New).is_some());
        assert!(lookup(T, Some("fs"), "Worker", CallStyle::New).is_none());
        let m = lookup(T, None, "include_str", CallStyle::Macro).unwrap();
        assert_eq!(m.phase, Phase::Build);
        assert!(table_has_module(T, "fs"));
        assert!(!table_has_module(T, "path"));
    }
}
