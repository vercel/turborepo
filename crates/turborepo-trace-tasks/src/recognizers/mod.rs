//! The compiled-in [`Recognizer`]s, by area.
//!
//! | Module | Programs |
//! |---|---|
//! | [`codegen`] | code generators (Prisma, graphql-codegen, buf/protoc, openapi, sqlc, go generate) |
//! | [`run`] | `node`, `tsx`, `ts-node`, `bun`, `deno run`, `python`, `go run` with a file |
//! | [`js`] | `tsc`, `next`, `vite`, `tsup`, `esbuild`, `rollup`, `webpack`, package managers' `run` |
//! | [`testrunners`] | `jest`, `vitest`, `pytest`, `go test`, `cargo test` |
//! | [`fs`] | `cp`, `cat`, `rsync`, `mkdir`, `rm`, `echo`, ... |
//! | [`bins`] | `go tool <x>`; npm package binaries and Python console scripts through the registry's unknown-program hook |
//!
//! [`util`] has argv scanning, path/glob and config-loading helpers for
//! recognizers.

pub mod bins;
pub mod codegen;
pub mod fs;
pub mod js;
pub mod run;
pub mod testrunners;
pub mod util;

use crate::registry::Recognizer;

/// Every specific recognizer, in priority order (code generators first).
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    let mut v: Vec<&'static dyn Recognizer> = Vec::new();
    v.extend(codegen::recognizers());
    v.extend(bins::recognizers());
    v.extend(js::recognizers());
    v.extend(testrunners::recognizers());
    v.extend(run::recognizers());
    v
}

/// Generic recognizers that come after every other (file utilities, no-op
/// builtins).
pub fn fallback_recognizers() -> Vec<&'static dyn Recognizer> {
    fs::recognizers()
}
