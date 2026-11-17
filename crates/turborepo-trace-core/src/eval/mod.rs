//! Shared partial-evaluation building blocks for the language plugins.
//!
//! - [`Value`]: the value model a plugin's expression evaluator produces for
//!   path arguments: a known string, a pattern with wildcards, capped
//!   alternatives, or unknown. [`Value`] supports concatenation and joining and
//!   converts to and from [`Specifier`].
//! - [`expand`]: glob expansion of a pattern against the core [`DirCache`],
//!   with a match limit. A leading wildcard is never expanded.
//! - [`resolve_pattern`]: a ready-made resolver step turning a pattern
//!   specifier into a [`Target::Glob`] plus the right [`Uncertainty`].
//! - [`path`]: POSIX path operations (join, normalize, dirname, basename,
//!   extname, relative) on [`Value`]s, shared by the language evaluators.
//! - [`anchor`]: anchored path values ([`anchor::Anchored`]: a language anchor
//!   such as `__dirname`, `__file__` or `CARGO_MANIFEST_DIR` plus a relative
//!   value) and the shared rule for which of them are emitted as assets
//!   ([`anchor::is_trivial`]).
//! - [`loader`]: loader wrappers ([`loader::Wrappers`]) and module strings
//!   ([`loader::object_ref`]).

pub mod anchor;
mod glob;
pub mod loader;
pub mod path;
mod value;

pub use glob::{ExpandOptions, Expansion, Refusal, expand, expand_with, resolve_pattern};
pub use value::{MAX_ALTERNATIVES, PatPart, Value};

#[doc(no_inline)]
pub use crate::{DirCache, Specifier, Target, Uncertainty};
