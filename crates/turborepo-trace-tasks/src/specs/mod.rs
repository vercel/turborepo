//! Declarative tool specs: recognizers written as data
//! (`crates/turborepo-trace-tasks/specs/*.toml`, compiled in with
//! `include_str!`) and interpreted by one generic [`Recognizer`]
//! ([`engine::SpecRecognizer`]).
//!
//! Use a spec when a tool's file effects follow from its flags, its config's
//! fields and defaults. Irreducible file-format parsing goes in a
//! [`crate::scanners`] scanner, which a spec names. Real control flow stays a
//! Rust recognizer.
//!
//! # Format
//!
//! ```toml
//! id = "sqlc"                       # recognizer id
//! programs = ["sqlc"]               # program base names
//! toolchain = ["go:github.com/sqlc-dev/sqlc/cmd/sqlc"]   # always added, even when ignored
//! env = []                          # env names always read
//! flags = ["--file|-f", "--against"]  # flags taking a value; first spelling is canonical
//! subcommand_split = ":"            # optional: `generate:pg` → `generate`
//! ignore = ["", "help", "version"]  # subcommands with no file effect ("" = none)
//!
//! [config]                          # optional; loaded before the rules run
//! label = "sqlc"                    # prefix of uncertainty details
//! flag = "--file"                   # explicit config file
//! names = ["sqlc.yaml", "sqlc.yml"] # else the first existing name...
//! search = "cwd"                    # ...in the cwd (or "package", or "up" to the repo root)
//! package_key = "sqlc"              # else this package.json key holds the config
//! probe_all = false                 # missing: probe every name (default: the first)
//! required = "sqlc without a config"      # missing is uncertain
//! unreadable = "sqlc config not readable" # unreadable is uncertain
//! require_any = ["sql", "packages"]       # a config without any of these is uncertain
//! key_flag = "--project"            # optional: select keys of a top-level `*`
//!
//! [values]                          # named values; sources in precedence order
//! sql = { field = ["sql[].schema", "sql[].queries"], optional = true }
//! out = { flag = "--out", field = "out", default = "drizzle", base = "cwd" }
//!
//! [groups]                          # named rule lists
//! read = [{ input = "sql", glob = "*.sql", reason = "sqlc sql" }]
//!
//! [sub]                             # rules per subcommand ("a|b", "migrate dev")
//! generate = ["read", { output = "out" }]
//! # or, without subcommands: [[variant]] when = ["@args"] config = false rules = [...]
//! ```
//!
//! **Values.** The first source that yields anything wins: `flag` (every
//! occurrence, relative to the cwd), `field` (key paths into the config,
//! relative to its directory), `pkg` (a key path into the nearest
//! `package.json`, which is recorded even when an earlier source wins),
//! `default` (a string, or candidates of which the first existing wins, with
//! the missing ones before it recorded as probes; a trailing `/` marks a
//! directory, probed as a `dir_glob` membership). `base = "cwd"` resolves every
//! source against the cwd; `default_base = "<value>"` resolves the default
//! against that value's directory. In rules, `"--flag"` and `"@args"`
//! (positionals after the subcommand) are values too.
//!
//! **Key paths.** `a.b` nested keys, `a[]` every array element, `*` every
//! object entry (`*!go` except `go`). A non-static part is an uncertainty
//! (`<label> `<path>`: <reason>`), with the config's directory as a membership.
//! A missing last key in an iterated element is an uncertainty unless the value
//! is `optional`; `required` makes an empty value one. `or_key = "target"`
//! reads `{ target }` when the leaf is an object; `base_field` resolves against
//! a field of the nearest enclosing object (orval's `workspace`); `env_refs`
//! collects `process.env.X` names instead of strings.
//!
//! **Rules.** One role each, applied to every item of a value:
//!
//! | Role | Effect | Modifiers |
//! |---|---|---|
//! | `input` | files are inputs, globs and directories memberships, URLs uncertain | `code` (JS/TS modules, also entries), `glob` (for directories), `reason` |
//! | `output` | outputs | `glob` (template below the path, `${key:-default}` reads enclosing config objects), `join`, `siblings`, `dir_when` |
//! | `membership` | memberships | `glob`, `join`, `reason` |
//! | `config` | config files (code configs also entries) | |
//! | `env` | env names | |
//! | `run` | command lines, recognized with the registry from the value's directory | |
//! | `scan` + `on` | a named [`crate::scanners`] scanner | |
//! | `toolchain` + `on` | toolchain packages of that ecosystem | |
//! | `tool` | one toolchain package `<ecosystem>:<name>` | |
//! | `uncertain` + `detail` | an uncertainty (`{sub}` = the subcommand) | |
//! | `use` | a group | `read_outputs` (its outputs become memberships with this reason) |
//!
//! Every rule may have `when` (all hold) and `unless` (none holds) conditions:
//! `--flag` (present), `@args` (positionals), `a|b` (either).
//!
//! **Flow.** Toolchain; then the subcommand (ignored → `Ignored`; unknown → an
//! `UnknownCommand` uncertainty with a package membership) or the first variant
//! whose `when` holds; the config; `always` rules, then the selected rules.
//! `Full` unless an uncertainty was added.

pub mod engine;
pub mod format;

use std::sync::OnceLock;

pub use engine::SpecRecognizer;

use crate::registry::Recognizer;

/// The compiled-in specs (file stem, TOML).
pub const SOURCES: &[(&str, &str)] = &[
    ("prisma", include_str!("../../specs/prisma.toml")),
    ("drizzle-kit", include_str!("../../specs/drizzle-kit.toml")),
    (
        "openapi-typescript",
        include_str!("../../specs/openapi-typescript.toml"),
    ),
    ("orval", include_str!("../../specs/orval.toml")),
    ("sqlc", include_str!("../../specs/sqlc.toml")),
];

/// Every spec recognizer, parsed once. Panics on an invalid spec (the
/// `specs_parse` test reports it first).
pub fn specs() -> &'static [SpecRecognizer] {
    static S: OnceLock<Vec<SpecRecognizer>> = OnceLock::new();
    S.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(name, src)| {
                SpecRecognizer::parse(src)
                    .unwrap_or_else(|e| panic!("crates/tasks/specs/{name}.toml: {e}"))
            })
            .collect()
    })
}

/// The spec recognizers, for the registry.
pub fn recognizers() -> Vec<&'static dyn Recognizer> {
    specs().iter().map(|s| s as &dyn Recognizer).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse() {
        for (name, src) in SOURCES {
            let s = SpecRecognizer::parse(src).unwrap_or_else(|e| panic!("{name}.toml: {e}"));
            assert_eq!(s.spec.id, *name, "the file stem is the id");
        }
    }

    #[test]
    fn every_file_is_compiled_in() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("specs");
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            assert!(
                SOURCES.iter().any(|(n, _)| *n == stem),
                "add specs/{stem}.toml to specs::SOURCES"
            );
        }
    }

    /// The worked examples in the crate's `README.md` ("Adding a tool to the
    /// task recognizers").
    #[test]
    fn documented_examples_parse() {
        let doc = include_str!("../../README.md");
        let section = doc
            .split("## Adding a tool to the task recognizers")
            .nth(1)
            .unwrap();
        let blocks: Vec<&str> = section
            .split("```toml\n")
            .skip(1)
            .map(|b| b.split("```").next().unwrap())
            .collect();
        assert!(blocks.len() >= 2, "examples found");
        for b in blocks.iter().take(2) {
            SpecRecognizer::parse(b).unwrap_or_else(|e| panic!("{e}\n{b}"));
        }
    }

    #[test]
    fn rejects_bad_specs() {
        for (src, err) in [
            ("id = \"x\"\nprograms = [\"x\"]\nbogus = 1", "unknown field"),
            (
                "id = \"x\"\nprograms = [\"x\"]\nalways = [{ input = \"nope\" }]",
                "unknown value",
            ),
            (
                "id = \"x\"\nprograms = [\"x\"]\nalways = [{ scan = \"nope\", on = \"@args\" }]",
                "unknown scanner",
            ),
            (
                "id = \"x\"\nprograms = [\"x\"]\nalways = [{ input = \"@args\", output = \
                 \"@args\" }]",
                "exactly one role",
            ),
        ] {
            let e = SpecRecognizer::parse(src).err().unwrap_or_default();
            assert!(e.contains(err), "{src}: {e}");
        }
    }
}
