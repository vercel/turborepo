# Rust fixtures

Inputs for `crates/turborepo-trace-lang-rust/tests/fixtures.rs` (snapshot tests), corpus seeds for
`crates/turborepo-trace-lang-rust/tests/conformance.rs`, and `cargo test` adapter inputs for
`crates/turborepo-trace-frameworks/tests/cargotest.rs`. Traces run for `x86_64-unknown-linux-gnu`.

## `basic/`: a workspace with a vendored registry crate

- `Cargo.toml`: virtual workspace (`members`, `exclude`, `[workspace.dependencies]`
  inherited by members), `Cargo.lock` v4, `.cargo/config.toml` replacing crates.io with
  `vendor/`.
- `app/src/main.rs` (binary):
  - `mod cli;` → `cli.rs`, a non-mod-rs file whose `mod args;` is `cli/args.rs`, whose inline
    `mod inner { mod deep; }` is `cli/args/inner/deep.rs`;
  - `mod net;` → `net/mod.rs` with `#[path = "tls_impl.rs"] mod tls;`; the `#[path]` file owns
    its directory, so its `mod session;` is `net/session.rs`;
  - `#[cfg(target_os = "linux")] #[path = …] mod sys;` (kept), `#[cfg(windows)] mod win;`
    (dropped: `win.rs` is never traced), `#[cfg(test)] mod tests;` (kept, CONDITIONAL);
  - `include_str!("../help.txt")`, `include!(concat!(env!("OUT_DIR"), …))` (GeneratedFile),
    a cwd-relative `std::fs::read_to_string` (CwdDependent) and a `CARGO_MANIFEST_DIR`-anchored
    read (`assets/logo.svg`).
- `app/build.rs`: `cargo:rerun-if-changed=proto/api.proto` and `build.rs`.
- `app/tests/integration.rs`: an integration test with `tests/common/mod.rs` and an embed.
- `util/`: a library with a `cfg(test)` module using insta (`src/snapshots/*.snap`).
- `vendor/regdep`: feature-gated modules (`std` enabled through the member's features);
  `vendor/optdep` is an optional dependency that is not enabled and never traced.
- `excluded/`: listed in `workspace.exclude`.

## `registry/`: a standalone package with a hermetic `$CARGO_HOME`

`cargo-home/` holds `registry/src/index.crates.io-*/<name>-<version>` crates and a git
checkout `git/checkouts/gitws-*/abcdef0/` (a git workspace with the package in
`crates/gitdep`). The package has an explicit `[lib]` and `[[bin]]`, a renamed dependency
(`renamed = { package = "dep-b" }`), a Windows-only `[target.'cfg(windows)'.dependencies]`
entry (skipped for Linux), a dev-dependency, a bench and an example.

## `xlang/`

Cross-language parity scenarios, each a standalone crate, also run by turbotrace's runtime
oracle: `assets` (`concat!(env!("CARGO_MANIFEST_DIR"), …)` in a
`const`, `Path::new(env!(…)).join(…)` in a `let`), `evaluator` (`format!`, `PathBuf::join`),
`discovery` (`fs::read_dir(concat!(…))`), `conditional` (`#[cfg]` constants), `loaders`
(`libloading::Library::new`, static only) and `platform` (`cargo:rustc-link-*` in `build.rs`).
