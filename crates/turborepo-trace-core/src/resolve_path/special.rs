//! Package special cases as data: when a known file of a package is
//! parsed, emit data directories or files that the package locates at runtime
//! in ways static analysis cannot see (in the style of PyInstaller hooks and
//! nft's `special-cases.ts`).
//!
//! The format is shared; tables and language-specific actions stay in the
//! language crate:
//!
//! ```ignore
//! use turborepo_trace_core::resolve_path::special::{Action::*, CaseConfig, FileCase, NoLang};
//!
//! const CASES: &[FileCase<NoLang>] = &[FileCase {
//!     package: "certifi",
//!     suffix: "certifi/core.py",
//!     actions: &[File("cacert.pem")],
//! }];
//! ```
//!
//! Paths in actions are relative to the triggering file's directory. [`apply`]
//! runs the common actions and hands [`Action::Lang`] back to the caller.
//! Placeholders such as `{os}`/`{arch}` are expanded by the caller's `subst`
//! (in its ecosystem's names); `Dir` paths are used verbatim.

use std::path::Path;

use crate::{
    ExtraEdge, RefKind, ResolveCx, Target,
    fs::normalize,
    resolve_path::assets::{AssetRules, keep_all, pattern_parts, special_spec},
};

/// No language-specific actions.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoLang {}

/// What a file-triggered case emits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Action<X: 'static> {
    /// Every asset below a directory (not placeholder-expanded).
    Dir(&'static str),
    /// One file (asset), if it exists.
    File(&'static str),
    /// One file followed as code ([`CaseConfig::dep_kind`]), if it exists.
    Dep(&'static str),
    /// Files matching a pattern with `*` wildcards ([`CaseConfig::glob_kind`]).
    Glob(&'static str),
    /// A language-specific action, handled by the caller.
    Lang(X),
}

/// A file-triggered case: package name, path suffix of the triggering file,
/// actions.
#[derive(Copy, Clone, Debug)]
pub struct FileCase<X: 'static> {
    /// Package the file belongs to.
    pub package: &'static str,
    /// Suffix of the file's path.
    pub suffix: &'static str,
    /// What to emit.
    pub actions: &'static [Action<X>],
}

impl<X> FileCase<X> {
    /// Whether the case applies to a file of package `package` at `path`.
    pub fn matches(&self, package: Option<&str>, path: &str) -> bool {
        package == Some(self.package) && path.ends_with(self.suffix)
    }
}

/// How common actions become edges.
#[derive(Copy, Clone, Debug)]
pub struct CaseConfig {
    /// Asset rules for directory and glob expansion.
    pub rules: AssetRules,
    /// Kind of [`Action::Dep`] edges (JS: `Require`).
    pub dep_kind: RefKind,
    /// Kind of [`Action::Glob`] edges (JS: `NativeAddon`).
    pub glob_kind: RefKind,
}

/// Runs a common action of package `pkg`'s case for the parsed file
/// `cx.from_path`, pushing edges labelled `<pkg>: <path>` (with
/// [`special_spec`]). Returns the payload of [`Action::Lang`] for the caller to
/// handle, else `None`.
pub fn apply<'a, X>(
    action: &'a Action<X>,
    pkg: &str,
    config: &CaseConfig,
    subst: &dyn Fn(&str) -> String,
    cx: &ResolveCx<'_>,
    out: &mut Vec<ExtraEdge>,
) -> Option<&'a X> {
    let dir = cx.from_dir;
    match action {
        Action::Dir(rel) => {
            let d = normalize(&dir.join(rel));
            if cx.fs.is_dir(&d) {
                out.push(config.rules.dir_edge(&d, cx, format!("{pkg}: {rel}/")));
            }
        }
        Action::File(rel) | Action::Dep(rel) => {
            let rel = subst(rel);
            let f = normalize(&dir.join(&rel));
            if cx.fs.is_file(&f) {
                let kind = if matches!(action, Action::Dep(_)) {
                    config.dep_kind
                } else {
                    RefKind::Asset
                };
                out.push(ExtraEdge::new(
                    kind,
                    Target::File(cx.intern(&f)),
                    special_spec(format!("{pkg}: {rel}")),
                ));
            }
        }
        Action::Glob(rel) => {
            let rel = subst(rel);
            let pat = normalize(&dir.join(&rel));
            let parts = pattern_parts(&pat.to_string_lossy());
            if let Some(g) = config.rules.glob(&parts, Path::new("/"), cx, keep_all)
                && !g.matches.is_empty()
            {
                out.push(ExtraEdge::new(
                    config.glob_kind,
                    g.into_target(cx),
                    special_spec(format!("{pkg}: {rel}")),
                ));
            }
        }
        Action::Lang(x) => return Some(x),
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_matching() {
        let c: FileCase<NoLang> = FileCase {
            package: "certifi",
            suffix: "certifi/core.py",
            actions: &[Action::File("cacert.pem")],
        };
        assert!(c.matches(Some("certifi"), "/v/site-packages/certifi/core.py"));
        assert!(!c.matches(Some("other"), "/v/site-packages/certifi/core.py"));
        assert!(!c.matches(None, "/v/site-packages/certifi/core.py"));
    }
}
