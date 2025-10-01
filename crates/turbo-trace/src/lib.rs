//! Utilities for tracing and analyzing import dependencies in JavaScript and
//! TypeScript files. Provides functionality to discover and track module
//! imports across a codebase. Used for `turbo boundaries`

#![deny(clippy::all)]
mod import_finder;
mod tracer;

pub use import_finder::{ImportFinder, ImportType};
pub use tracer::{ImportTraceType, TraceError, TraceResult, Tracer};
