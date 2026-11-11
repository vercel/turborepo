//! Lockfile parsing and diffing for `turbo trace`: opaque third-party
//! packages and shard keys.
//!
//! Every supported format parses into one neutral [`Lockfile`]:
//! [`LockedPackage`]s keyed by a format-specific id, and [`Importer`]s
//! (workspace packages) with their direct dependencies. [`diff`] compares two
//! lockfiles and reports added/removed/changed packages, per-importer
//! direct-dependency changes, and the identity-changed and closure-changed
//! package sets.
//!
//! | Format | File | Versions |
//! |---|---|---|
//! | pnpm | `pnpm-lock.yaml` | lockfileVersion 6.x (pnpm 8), 9.0 (pnpm 9, 10) |
//! | npm | `package-lock.json`, `npm-shrinkwrap.json` | lockfileVersion 2, 3 |
//! | yarn classic | `yarn.lock` | v1 |
//! | yarn berry | `yarn.lock` | `__metadata.version` 4–8 (yarn 2–4) |
//! | bun | `bun.lock` | lockfileVersion 0–2 (text); binary `bun.lockb` is not supported |
//! | go | `go.mod` + `go.sum` ([`gosum`]) | any `go` directive |
//! | cargo | `Cargo.lock` ([`cargo`]) | `version` 1–4 |
//!
//! ```
//! use turborepo_trace_lockfile::{parse, LockfileKind};
//! let src = "lockfileVersion: '9.0'\nimporters:\n  .:\n    dependencies:\n      ms:\n        specifier: ^2.1.0\n        version: 2.1.3\npackages:\n  ms@2.1.3:\n    resolution: {integrity: sha512-x}\nsnapshots:\n  ms@2.1.3: {}\n";
//! let lock = parse(LockfileKind::Pnpm, src).unwrap();
//! let ms = lock.resolve(".", "ms").unwrap();
//! assert!(matches!(ms, turborepo_trace_lockfile::Resolved::Package(p) if p.version == "2.1.3"));
//! ```

// Every public item is documented.
#![warn(missing_docs)]

mod bun;
pub mod cargo;
mod diff;
pub mod gosum;
mod json;
mod model;
mod npm;
mod pnpm;
mod util;
mod yaml;
mod yarn_berry;
mod yarn_classic;

use std::path::Path;

pub use diff::{
    AffectedPackages, ChangedFields, DirectDep, DirectDepChange, ImporterDiff, ImporterStatus,
    LockDiff, PackageChange, VersionChange, diff,
};
pub use model::{
    CatalogEntry, DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat,
    LockfileMeta, PackageFlags, Platform, Resolved, Target, normalize_importer_path,
};

/// Errors from parsing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Malformed input.
    #[error("{format} lockfile: line {line}: {message}")]
    Syntax {
        /// Format being parsed.
        format: LockfileFormat,
        /// 1-based line.
        line: usize,
        /// What went wrong.
        message: String,
    },
    /// Malformed JSON.
    #[error("{format} lockfile: {source}")]
    Json {
        /// Format being parsed.
        format: LockfileFormat,
        /// The JSON error (with line and column).
        source: serde_json::Error,
    },
    /// A lockfile version this crate does not read.
    #[error("{format} lockfile: unsupported lockfile version {version}")]
    UnsupportedVersion {
        /// Format being parsed.
        format: LockfileFormat,
        /// The version found.
        version: String,
    },
    /// A file that is not a supported lockfile (for example the binary
    /// `bun.lockb`).
    #[error("unsupported lockfile: {0}")]
    Unsupported(String),
    /// Reading the file failed.
    #[error("reading {path}: {source}")]
    Io {
        /// The path.
        path: String,
        /// The I/O error.
        source: std::io::Error,
    },
}

/// Result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Which parser to use. Yarn classic and berry share a file name, so [`detect`]
/// looks at the contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LockfileKind {
    /// `pnpm-lock.yaml`.
    Pnpm,
    /// `package-lock.json`.
    Npm,
    /// Yarn v1 `yarn.lock`.
    YarnClassic,
    /// Yarn v2+ `yarn.lock`.
    YarnBerry,
    /// `bun.lock`.
    Bun,
    /// Go `go.mod` (requirements without hashes; see [`gosum::parse`] for
    /// `go.mod` plus `go.sum`).
    GoMod,
    /// Go `go.sum` on its own (every listed module version).
    GoSum,
    /// Cargo `Cargo.lock`.
    Cargo,
}

/// A Python distribution name in its PEP 503 form: lowercase, runs of `-`,
/// `_` and `.` become one `-` (how a project names itself in `pypi:<name>`).
pub fn pep503_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut sep = false;
    for c in name.trim().chars() {
        if matches!(c, '-' | '_' | '.') {
            sep = true;
        } else {
            if sep && !out.is_empty() {
                out.push('-');
            }
            sep = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// Detects the lockfile kind from its file name and, for `yarn.lock` (and
/// unknown names), its contents. Returns an error for `bun.lockb` and
/// unrecognized files.
pub fn detect(file_name: &str, contents: &str) -> Result<LockfileKind> {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    match base {
        "pnpm-lock.yaml" => return Ok(LockfileKind::Pnpm),
        "package-lock.json" | "npm-shrinkwrap.json" => return Ok(LockfileKind::Npm),
        "bun.lock" => return Ok(LockfileKind::Bun),
        "go.mod" => return Ok(LockfileKind::GoMod),
        "go.sum" => return Ok(LockfileKind::GoSum),
        "Cargo.lock" => return Ok(LockfileKind::Cargo),
        "bun.lockb" => {
            return Err(Error::Unsupported(
                "bun.lockb is binary; run `bun install --save-text-lockfile`".into(),
            ));
        }
        _ => {}
    }
    let head = &contents[..contents.len().min(4096)];
    if head.contains("__metadata:") {
        Ok(LockfileKind::YarnBerry)
    } else if head.contains("# yarn lockfile v1") || base == "yarn.lock" {
        Ok(LockfileKind::YarnClassic)
    } else if head.contains("lockfileVersion:") {
        Ok(LockfileKind::Pnpm)
    } else if head.trim_start().starts_with('{')
        && head.contains("\"workspaces\"")
        && !head.contains("\"requires\"")
    {
        Ok(LockfileKind::Bun)
    } else if head.contains("\"lockfileVersion\"") {
        Ok(LockfileKind::Npm)
    } else {
        Err(Error::Unsupported(format!(
            "cannot detect lockfile format of {file_name}"
        )))
    }
}

/// Parses a lockfile of a known kind.
pub fn parse(kind: LockfileKind, contents: &str) -> Result<Lockfile> {
    match kind {
        LockfileKind::Pnpm => pnpm::parse(contents),
        LockfileKind::Npm => npm::parse(contents),
        LockfileKind::YarnClassic => yarn_classic::parse(contents),
        LockfileKind::YarnBerry => yarn_berry::parse(contents),
        LockfileKind::Bun => bun::parse(contents),
        LockfileKind::GoMod => gosum::parse(contents, None),
        LockfileKind::GoSum => gosum::parse_go_sum(contents).map(|s| gosum::go_sum_lockfile(&s)),
        LockfileKind::Cargo => cargo::parse(contents),
    }
}

/// Detects the kind from `file_name` and contents, then parses.
pub fn parse_named(file_name: &str, contents: &str) -> Result<Lockfile> {
    parse(detect(file_name, contents)?, contents)
}

/// Reads and parses a lockfile from disk.
pub fn parse_path(path: impl AsRef<Path>) -> Result<Lockfile> {
    let path = path.as_ref();
    let display = path.display().to_string();
    if path.file_name().is_some_and(|n| n == "bun.lockb") {
        return Err(Error::Unsupported(
            "bun.lockb is binary; run `bun install --save-text-lockfile`".into(),
        ));
    }
    let contents = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: display.clone(),
        source,
    })?;
    parse_named(&display, &contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pep503_names() {
        assert_eq!(pep503_name("Friendly-Bard"), "friendly-bard");
        assert_eq!(pep503_name("FRIENDLY._.bard"), "friendly-bard");
        assert_eq!(pep503_name("typing_extensions"), "typing-extensions");
    }
}
