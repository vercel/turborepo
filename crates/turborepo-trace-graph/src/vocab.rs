//! Edge and uncertainty vocabulary.
//!
//! These mirror `turborepo_trace_core::{RefKind, Phase, RefFlags,
//! UncertaintyKind}`. They are not re-exported from core, for two reasons:
//!
//! - **Build isolation.** Depending on `turborepo-trace-core` would make this
//!   crate rebuild with every change to core, and would pull core's whole
//!   dependency tree into a crate that only needs five enums.
//! - **The names are the contract.** Every graph format (JSON, SQLite, the
//!   shard store's symbolic edges) carries these as stable snake_case strings,
//!   and core already exposes the same strings through `as_str()`. Converting
//!   is `RefKind::parse(core_kind.as_str())`, which a unit test in the store
//!   crate can pin against `turborepo_trace_core::RefKind::ALL`.
//!
//! The query engine packs kind, phase and flags of an edge into a 16-bit
//! [`EdgeAttr`], which lives next to the target in the adjacency arrays so that
//! edge filters cost one mask test.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $( $(#[$vmeta:meta])* $variant:ident = $s:literal, )* }
    ) => {
        $(#[$meta])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $( $(#[$vmeta])* $variant, )* }

        impl $name {
            /// Every value, in declaration order (the order of the packed discriminant).
            pub const ALL: &'static [$name] = &[$($name::$variant),*];

            /// Stable snake_case name, identical to `turborepo_trace_core`'s `as_str()`.
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s,)* }
            }

            /// Parses a stable name.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$variant),)* _ => None }
            }

            /// All stable names, for error messages.
            pub fn names() -> impl Iterator<Item = &'static str> {
                Self::ALL.iter().map(|v| v.as_str())
            }

            #[inline]
            #[allow(dead_code)]
            pub(crate) const fn index(self) -> u8 {
                self as u8
            }

            #[inline]
            #[allow(dead_code)]
            pub(crate) fn from_index(i: u8) -> Self {
                Self::ALL[i as usize]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

str_enum! {
    /// What a reference means to the program that contains it (core `RefKind`).
    pub enum RefKind {
        /// Static import.
        Import = "import",
        /// CommonJS `require()`.
        Require = "require",
        /// `import()` and other lazy module loads.
        DynamicImport = "dynamic_import",
        /// `export ... from`.
        ReExport = "re_export",
        /// Go package fan-out.
        Package = "package",
        /// Rust `mod x;`.
        Mod = "mod",
        /// Compile-time embedding.
        Embed = "embed",
        /// Runtime file read.
        FsRead = "fs_read",
        /// Runtime directory read.
        FsReadDir = "fs_read_dir",
        /// Worker script.
        Worker = "worker",
        /// Child process script.
        ChildProcess = "child_process",
        /// Native addon.
        NativeAddon = "native_addon",
        /// Shared library.
        SharedLib = "shared_lib",
        /// Manifest consulted during resolution.
        Manifest = "manifest",
        /// Static asset referenced by location.
        Asset = "asset",
    }
}

impl RefKind {
    /// The phase a reference of this kind has unless stated otherwise (as in
    /// core).
    pub const fn default_phase(self) -> Phase {
        match self {
            RefKind::Embed | RefKind::Mod | RefKind::Package => Phase::Build,
            _ => Phase::Runtime,
        }
    }
}

str_enum! {
    /// When a file is needed (core `Phase`).
    pub enum Phase {
        /// Needed to run the program.
        Runtime = "runtime",
        /// Needed only to compile it.
        Build = "build",
    }
}

str_enum! {
    /// Categories of uncertainty (core `UncertaintyKind`).
    pub enum UncertaintyKind {
        /// `require(x)`: nothing known.
        ComputedSpecifier = "computed_specifier",
        /// `require('./a/' + x)`: narrowed to a glob.
        PartialSpecifier = "partial_specifier",
        /// Glob over the match limit, truncated.
        PatternTooBroad = "pattern_too_broad",
        /// Literal, but resolution failed.
        UnresolvedSpecifier = "unresolved_specifier",
        /// Aliased or shadowed loader.
        AliasedLoader = "aliased_loader",
        /// `eval`, `new Function`, `exec`, proc macros.
        EvalLike = "eval_like",
        /// Environment-dependent path.
        EnvDependent = "env_dependent",
        /// Relative fs path not anchored to the file.
        CwdDependent = "cwd_dependent",
        /// Exports conditions, build tags, cfg.
        ConditionalResolution = "conditional_resolution",
        /// Generated file.
        GeneratedFile = "generated_file",
        /// Resolves only case-insensitively.
        CaseMismatch = "case_mismatch",
        /// Lexer error recovery or unreadable file.
        ParseIncomplete = "parse_incomplete",
        /// Construct deliberately not modelled.
        UnsupportedSyntax = "unsupported_syntax",
        /// Resolved outside the trace base.
        OutsideBase = "outside_base",
        /// A build-task command no recognizer knows.
        UnknownCommand = "unknown_command",
        /// Shell syntax in a task command that is not modelled.
        ShellUnsupported = "shell_unsupported",
    }
}

str_enum! {
    /// What an uncertain reference does with its target. Derived from the kind of the
    /// reference that raised the uncertainty.
    pub enum Effect {
        /// Not raised by a reference (file-level diagnostics, tasks), or not recorded.
        None = "none",
        /// Loads the target as code: imports, `require`, workers, child processes, native
        /// addons. The target's own dependencies matter.
        Load = "load",
        /// Reads the target's contents: fs reads, directory listings, asset paths,
        /// embeds. Only the target itself matters.
        Read = "read",
    }
}

impl Effect {
    /// The effect of a reference kind.
    pub fn of(kind: RefKind) -> Effect {
        match kind {
            RefKind::FsRead | RefKind::FsReadDir | RefKind::Asset | RefKind::Embed => Effect::Read,
            RefKind::Manifest => Effect::None,
            _ => Effect::Load,
        }
    }
}

str_enum! {
    /// Where the unknown part of an uncertain reference's value comes from (core `Origin`).
    pub enum Origin {
        /// Not analyzed, or the analysis could not tell.
        Unknown = "unknown",
        /// A function parameter bound by callers the file's analysis can't see.
        Param = "param",
        /// The environment.
        Env = "env",
        /// The process working directory.
        Cwd = "cwd",
        /// The command line.
        Argv = "argv",
        /// A binding imported from another module.
        Imported = "imported",
        /// The result of a call the analysis can't evaluate.
        Call = "call",
        /// Data only known at runtime.
        Data = "data",
    }
}

/// Qualifiers on a reference (core `RefFlags`), as a bit set.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RefFlags(u8);

impl RefFlags {
    /// No flags.
    pub const NONE: RefFlags = RefFlags(0);
    /// Failure is tolerated (`try`/`catch`).
    pub const OPTIONAL: RefFlags = RefFlags(1 << 0);
    /// Guarded by cfg, build tags, a ternary branch, ...
    pub const CONDITIONAL: RefFlags = RefFlags(1 << 1);
    /// Dynamic or function-local import.
    pub const LAZY: RefFlags = RefFlags(1 << 2);
    /// `import type`, `if TYPE_CHECKING:`.
    pub const TYPE_ONLY: RefFlags = RefFlags(1 << 3);
    /// The source reads or compiles the target as an input and does not execute
    /// it (a build task's input files and membership hubs). Uncertainty
    /// does not propagate along such an
    /// edge ([`EdgeFilter::for_uncertainty`]); changes do.
    pub const DATA: RefFlags = RefFlags(1 << 4);
    /// The reference locates a path without a tracked call reading it (core
    /// `LOCATED`: `new URL(x, import.meta.url)`). Never a byte read. Not a
    /// named flag. (Core's `NAMES`, bit 5 there, is not carried over.)
    pub const LOCATED: RefFlags = RefFlags(1 << 5);
    /// A file reads the bytes of another file through a tracked read call
    /// (`fs_read`, `fs_read_dir`), and nothing regenerates the target: what
    /// the reader sees changes only when the target itself is modified,
    /// added or deleted. Set by the graph store on its traversal adjacency,
    /// never by an extractor. Entries evaluated in task mode
    /// ([`crate::taskmode`]) follow such an edge only from a changed
    /// target; every other evaluation ignores the flag. Not a named flag:
    /// outputs and the `flags` filter don't show it.
    pub const BYTES: RefFlags = RefFlags(1 << 6);
    /// A generated-file link that exists only because the source lists a
    /// directory above the task's output with a pattern that reaches the
    /// output through wildcards alone (`**`, `*/*`): the source would see
    /// the produced files if they were there, but does not name them. Set
    /// by the graph store; skipped in task mode, followed everywhere
    /// else. Not a named flag.
    pub const LISTED: RefFlags = RefFlags(1 << 7);

    const NAMED: [(RefFlags, &'static str); 5] = [
        (RefFlags::OPTIONAL, "optional"),
        (RefFlags::CONDITIONAL, "conditional"),
        (RefFlags::LAZY, "lazy"),
        (RefFlags::TYPE_ONLY, "type_only"),
        (RefFlags::DATA, "data"),
    ];

    /// The raw bits (same bit positions as core).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// From raw bits; unknown bits are dropped (core's bit 5, `NAMES`, is not a
    /// query flag).
    pub const fn from_bits_truncate(bits: u8) -> Self {
        RefFlags(bits)
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: RefFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any flag in `other` is set.
    pub const fn intersects(self, other: RefFlags) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Parses one flag name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMED.iter().find(|(_, n)| *n == s).map(|(f, _)| *f)
    }

    /// Names of the set flags, in bit order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMED
            .into_iter()
            .filter(move |(f, _)| self.contains(*f))
            .map(|(_, n)| n)
    }

    /// All flag names, for error messages.
    pub fn all_names() -> impl Iterator<Item = &'static str> {
        Self::NAMED.into_iter().map(|(_, n)| n)
    }
}

impl std::ops::BitOr for RefFlags {
    type Output = RefFlags;
    fn bitor(self, rhs: RefFlags) -> RefFlags {
        RefFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for RefFlags {
    fn bitor_assign(&mut self, rhs: RefFlags) {
        self.0 |= rhs.0;
    }
}

impl Serialize for RefFlags {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.names())
    }
}

impl<'de> Deserialize<'de> for RefFlags {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = RefFlags;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of flag names")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<RefFlags, A::Error> {
                let mut flags = RefFlags::NONE;
                while let Some(name) = seq.next_element::<std::borrow::Cow<'de, str>>()? {
                    flags |= RefFlags::parse(&name).ok_or_else(|| {
                        serde::de::Error::unknown_variant(
                            &name,
                            &["optional", "conditional", "lazy", "type_only"],
                        )
                    })?;
                }
                Ok(flags)
            }
        }
        d.deserialize_seq(V)
    }
}

/// Kind, phase and flags of one edge, packed into 16 bits.
///
/// Layout: bits 0..4 kind, bit 4 phase (set = build), bit 5 the configuration
/// mark ([`EdgeAttr::is_config`]), bits 8..15 flags.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeAttr(pub u16);

impl EdgeAttr {
    const PHASE_BIT: u16 = 1 << 4;
    const CONFIG_BIT: u16 = 1 << 5;

    /// The same edge, marked as leading to the configuration of the tool that
    /// runs the source (a test's runner config; core `RefFlags::CONFIG`):
    /// loaded by the tool, not by the source's code. Changes and
    /// uncertainty propagate; path names do not ([`crate::names`]). Not one
    /// of [`RefFlags`]: the flag byte is full, and nothing filters by it.
    pub const fn config(self) -> Self {
        EdgeAttr(self.0 | Self::CONFIG_BIT)
    }

    /// Whether the edge leads to the configuration of the tool that runs the
    /// source ([`EdgeAttr::config`]).
    #[inline]
    pub const fn is_config(self) -> bool {
        self.0 & Self::CONFIG_BIT != 0
    }

    /// Packs the attributes.
    pub const fn new(kind: RefKind, phase: Phase, flags: RefFlags) -> Self {
        let p = match phase {
            Phase::Runtime => 0,
            Phase::Build => Self::PHASE_BIT,
        };
        EdgeAttr(kind.index() as u16 | p | ((flags.bits() as u16) << 8))
    }

    /// The reference kind.
    #[inline]
    pub fn kind(self) -> RefKind {
        RefKind::from_index((self.0 & 0x0f) as u8)
    }

    /// The phase.
    #[inline]
    pub fn phase(self) -> Phase {
        if self.0 & Self::PHASE_BIT != 0 {
            Phase::Build
        } else {
            Phase::Runtime
        }
    }

    /// The flags.
    #[inline]
    pub fn flags(self) -> RefFlags {
        RefFlags::from_bits_truncate((self.0 >> 8) as u8)
    }
}

/// A predicate over [`EdgeAttr`]: allowed kinds, allowed phases, and flags that
/// exclude an edge. Used to restrict traversal (for example `--runtime-only`
/// drops type-only edges) and by the `kind`/`phase`/`flags` filters.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct EdgeFilter {
    /// Bit `k` set means kind index `k` passes.
    pub kinds: u16,
    /// Bit 0 runtime, bit 1 build.
    pub phases: u8,
    /// Edges with any of these flags are rejected.
    pub exclude_flags: RefFlags,
    /// If non-empty, edges must have at least one of these flags.
    pub require_any_flag: RefFlags,
}

impl Default for EdgeFilter {
    fn default() -> Self {
        Self::ALL
    }
}

impl EdgeFilter {
    /// Accepts every edge.
    pub const ALL: EdgeFilter = EdgeFilter {
        kinds: u16::MAX,
        phases: 0b11,
        exclude_flags: RefFlags::NONE,
        require_any_flag: RefFlags::NONE,
    };

    /// Whether this filter accepts every edge.
    pub fn is_all(&self) -> bool {
        *self == Self::ALL
    }

    /// Only the given kinds.
    pub fn kinds(kinds: impl IntoIterator<Item = RefKind>) -> Self {
        let mut f = Self::ALL;
        f.kinds = kinds.into_iter().fold(0, |m, k| m | (1 << k.index()));
        f
    }

    /// Only the given phases.
    pub fn phases(phases: impl IntoIterator<Item = Phase>) -> Self {
        let mut f = Self::ALL;
        f.phases = phases.into_iter().fold(0, |m, p| m | (1 << p.index()));
        f
    }

    /// Drops edges carrying any of `flags` (for example
    /// [`RefFlags::TYPE_ONLY`]).
    pub fn without_flags(mut self, flags: RefFlags) -> Self {
        self.exclude_flags |= flags;
        self
    }

    /// The filter for propagating uncertainty: this filter without
    /// [`RefFlags::DATA`] edges. An entry inherits the open uncertainty of
    /// what it (transitively) executes. A build task reads or compiles its
    /// inputs; it does not run their dynamic loads and file
    /// reads, so uncertainty inside an input file says nothing about the task's
    /// outputs.
    pub fn for_uncertainty(self) -> Self {
        self.without_flags(RefFlags::DATA)
    }

    /// Whether `attr` passes.
    #[inline]
    pub fn accepts(&self, attr: EdgeAttr) -> bool {
        let kind_ok = self.kinds & (1 << (attr.0 & 0x0f)) != 0;
        let phase_ok = self.phases & (1 << ((attr.0 >> 4) & 1)) != 0;
        let flags = attr.flags();
        kind_ok
            && phase_ok
            && !flags.intersects(self.exclude_flags)
            && (self.require_any_flag.is_empty() || flags.intersects(self.require_any_flag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for k in RefKind::ALL {
            assert_eq!(RefKind::parse(k.as_str()), Some(*k));
            assert_eq!(RefKind::from_index(k.index()), *k);
        }
        for k in UncertaintyKind::ALL {
            assert_eq!(UncertaintyKind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(RefKind::ALL.len(), 15);
        assert_eq!(UncertaintyKind::ALL.len(), 16);
    }

    #[test]
    fn attr_packing() {
        for &k in RefKind::ALL {
            for &p in Phase::ALL {
                let f = RefFlags::LAZY | RefFlags::TYPE_ONLY;
                let a = EdgeAttr::new(k, p, f);
                assert_eq!((a.kind(), a.phase(), a.flags()), (k, p, f));
            }
        }
        let a = EdgeAttr::new(RefKind::Import, Phase::Runtime, RefFlags::TYPE_ONLY);
        assert!(EdgeFilter::ALL.accepts(a));
        assert!(
            !EdgeFilter::ALL
                .without_flags(RefFlags::TYPE_ONLY)
                .accepts(a)
        );
        assert!(!EdgeFilter::kinds([RefKind::Require]).accepts(a));
        assert!(!EdgeFilter::phases([Phase::Build]).accepts(a));
    }

    #[test]
    fn flags_serde() {
        let f: RefFlags = serde_json::from_str(r#"["lazy","optional"]"#).unwrap();
        assert_eq!(f, RefFlags::LAZY | RefFlags::OPTIONAL);
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"["optional","lazy"]"#);
        assert!(serde_json::from_str::<RefFlags>(r#"["nope"]"#).is_err());
    }
}
