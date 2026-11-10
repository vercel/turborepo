//! Named content scanners: the irreducible parsing of tool-specific file
//! formats, shared by declarative tool specs ([`crate::specs`], `scan =
//! "<name>"`) and Rust recognizers.
//!
//! A scanner gets one location (a value of a spec: a schema path, an API spec)
//! and adds what the file's contents imply to the task spec: referenced files,
//! outputs, env names.
//!
//! | Name | Module | What it reads |
//! |---|---|---|
//! | `refs` | [`refs`] | OpenAPI / JSON Schema `$ref`s and GraphQL `#import`s, transitively; URLs are uncertain |
//! | `prisma-schema` | [`prisma`] | a Prisma schema file or folder → schema inputs |
//! | `prisma-generators` | [`prisma`] | `generator` blocks → outputs, output packages, toolchain |
//! | `prisma-env` | [`prisma`] | `datasource` blocks → `env("X")` names |
//! | `prisma-format` | [`prisma`] | the schema files as outputs |
//!
//! To add one: write `fn(&RecognizeCx, &Target, &mut TaskSpec)` in a module
//! here and add a row to [`SCANNERS`].

pub mod prisma;
pub mod refs;

use std::path::{Path, PathBuf};

use turborepo_trace_frameworks::fsx;

use crate::{registry::RecognizeCx, spec::TaskSpec};

/// What a scanner scans: a location as written, with the directory it is
/// relative to.
pub struct Target<'a> {
    /// The location as written (a path, a glob or a URL).
    pub text: &'a str,
    /// The directory `text` is relative to.
    pub base: &'a Path,
    /// The directory that becomes a conservative membership when something is
    /// not statically known (the config's directory, else the cwd).
    pub ctx_dir: &'a Path,
}

impl Target<'_> {
    /// `text` resolved against `base`.
    pub fn path(&self) -> PathBuf {
        fsx::join(self.base, self.text)
    }
}

/// A scanner function.
pub type ScanFn = fn(&RecognizeCx<'_>, &Target<'_>, &mut TaskSpec);

/// A named scanner.
pub struct Scanner {
    /// The name specs use.
    pub name: &'static str,
    /// The function.
    pub scan: ScanFn,
}

/// Every scanner.
pub const SCANNERS: &[Scanner] = &[
    Scanner {
        name: "refs",
        scan: |_, t, out| {
            refs::add_spec(t.base, t.text, t.ctx_dir, out);
        },
    },
    Scanner {
        name: "prisma-schema",
        scan: prisma::scan_schema,
    },
    Scanner {
        name: "prisma-generators",
        scan: prisma::scan_generators,
    },
    Scanner {
        name: "prisma-env",
        scan: prisma::scan_env,
    },
    Scanner {
        name: "prisma-format",
        scan: prisma::scan_format,
    },
];

/// The scanner named `name`.
pub fn get(name: &str) -> Option<&'static Scanner> {
    SCANNERS.iter().find(|s| s.name == name)
}
