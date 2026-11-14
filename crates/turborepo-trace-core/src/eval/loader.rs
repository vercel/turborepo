//! Loader wrappers and module strings, shared by the language recognizers.
//!
//! Two general ideas that first appeared in the JS plugin:
//!
//! - A *loader wrapper* is a function that passes one of its parameters
//!   straight to a loader API (`const load = n => require(n)`, `def load(n):
//!   return importlib.import_module(n)`). Calls of the wrapper are references
//!   of the loader's kind, each with an [`UncertaintyKind::AliasedLoader`]
//!   record. [`Wrappers`] collects them per file with a cap; the language finds
//!   definitions and call sites in its own syntax. The loaders are the entries
//!   with `turborepo_trace_core::api::ArgRole::LoaderWrapper` in its API table.
//! - A *module string* is a string literal that names a module some library
//!   loads later (pino `target`, Django `INSTALLED_APPS`, setuptools entry
//!   points). [`object_ref`] splits the common `module:attr` notation; the
//!   language turns the module into an optional reference with an
//!   [`UncertaintyKind::AliasedLoader`] record.

use crate::{Span, Uncertainty, UncertaintyKind};

/// Maximum loader wrappers tracked per file.
pub const MAX_WRAPPERS: usize = 8;

/// One loader wrapper: a function name and the index of the parameter it passes
/// on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wrapper<N> {
    /// The function's name.
    pub name: N,
    /// The index of the parameter passed to the loader.
    pub param: usize,
}

/// The loader wrappers of one file (at most [`MAX_WRAPPERS`]).
#[derive(Clone, Debug)]
pub struct Wrappers<N> {
    list: Vec<Wrapper<N>>,
    /// The loader's name as written in messages (`require`,
    /// `importlib.import_module`).
    loader: &'static str,
    /// What the wrapper is called in messages (`require wrapper`).
    label: &'static str,
}

impl<N: AsRef<str> + PartialEq> Wrappers<N> {
    /// An empty set for `loader`, described as `label` in uncertainty records.
    pub fn new(loader: &'static str, label: &'static str) -> Self {
        Wrappers {
            list: Vec::new(),
            loader,
            label,
        }
    }

    /// Adds `name` passing parameter `param` on. Ignores the loader itself and
    /// duplicates. Returns `false` once the set is full.
    pub fn add(&mut self, name: N, param: usize) -> bool {
        if self.is_full() {
            return false;
        }
        if name.as_ref() != self.loader
            && !self.list.iter().any(|w| w.name == name && w.param == param)
        {
            self.list.push(Wrapper { name, param });
        }
        !self.is_full()
    }

    /// Whether no more wrappers are accepted.
    pub fn is_full(&self) -> bool {
        self.list.len() >= MAX_WRAPPERS
    }

    /// The parameter index of wrapper `name`.
    pub fn param_of(&self, name: &str) -> Option<usize> {
        self.list
            .iter()
            .find(|w| w.name.as_ref() == name)
            .map(|w| w.param)
    }

    /// Whether there are no wrappers.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The wrappers in discovery order.
    pub fn iter(&self) -> impl Iterator<Item = &Wrapper<N>> {
        self.list.iter()
    }

    /// Consumes the set into its wrappers.
    pub fn into_vec(self) -> Vec<Wrapper<N>> {
        self.list
    }

    /// The record text for calls of wrapper `name`.
    pub fn detail(&self, name: &str) -> String {
        format!(
            "`{name}` passes its parameter to `{}` ({})",
            self.loader, self.label
        )
    }

    /// The [`UncertaintyKind::AliasedLoader`] record for a call of wrapper
    /// `name` at `span`.
    pub fn uncertainty(&self, name: &str, span: Span) -> Uncertainty {
        Uncertainty::new(UncertaintyKind::AliasedLoader, span, self.detail(name))
    }
}

/// Splits an object reference `module:attr` (setuptools entry points, uvicorn,
/// gunicorn) into the module and the attribute. A trailing `[extras]` is
/// dropped. `None` for an empty module.
pub fn object_ref(s: &str) -> Option<(&str, Option<&str>)> {
    let s = s.split('[').next().unwrap_or(s).trim();
    let (m, attr) = match s.split_once(':') {
        Some((m, a)) => (m.trim(), Some(a.trim()).filter(|a| !a.is_empty())),
        None => (s, None),
    };
    (!m.is_empty()).then_some((m, attr))
}

/// Whether `s` looks like a dotted module path (`a.b_c.d`): identifier segments
/// separated by single dots. Used to tell module strings from other strings in
/// registries.
pub fn is_dotted_name(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|seg| {
            let mut b = seg.bytes();
            b.next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrappers() {
        let mut w: Wrappers<&str> = Wrappers::new("require", "require wrapper");
        assert!(w.add("load", 0));
        w.add("load", 0);
        w.add("require", 0);
        assert_eq!(w.iter().count(), 1);
        assert_eq!(w.param_of("load"), Some(0));
        assert_eq!(
            w.detail("load"),
            "`load` passes its parameter to `require` (require wrapper)"
        );
        for i in 0..20 {
            w.add(["a", "b", "c", "d", "e", "f", "g", "h", "i"][i % 9], i);
        }
        assert!(w.is_full());
        assert_eq!(w.iter().count(), MAX_WRAPPERS);
    }

    #[test]
    fn module_strings() {
        assert_eq!(object_ref("pkg.cli:main"), Some(("pkg.cli", Some("main"))));
        assert_eq!(
            object_ref("pkg.cli:main [extra]"),
            Some(("pkg.cli", Some("main")))
        );
        assert_eq!(object_ref("pkg"), Some(("pkg", None)));
        assert_eq!(object_ref(":x"), None);
        assert!(is_dotted_name("django.contrib.admin"));
        assert!(!is_dotted_name("a..b"));
        assert!(!is_dotted_name("templates/x.html"));
    }
}
