# Go fixtures

Inputs for `crates/turborepo-trace-lang-go/tests/{fixtures,graph}.rs` (snapshot and graph-mode tests), corpus
seeds for `crates/turborepo-trace-lang-go/tests/conformance.rs`, and the `go test` adapter tests in
`crates/turborepo-trace-frameworks/src/gotest.rs`. The tests pin `GOOS/GOARCH`, cgo and the module cache, and
disable `GOROOT` lookup, so results are machine independent. None of these modules builds with
the real `go` command (the module cache is fake); the `go list` differential runs on real
corpora instead (`cargo run -p turbotrace-lang-go --example go_corpus -- diff <dir> ./...`).

| Case | Exercises |
|---|---|
| `basic/` | module `example.com/basic`: stdlib imports, an aliased import, `_ "embed"`; `//go:embed` with a glob (`static/*` matches `.keep` directly), a directory (`tmpl`, where `_draft.tmpl` and `.hidden` are skipped) and a pattern without matches; `os.ReadFile` relative to the cwd; package mates (`version.go`); `internal/util` with `_linux`, `_windows`, `_darwin_arm64` files, a `//go:build integration` file, a `//go:build ignore` file, per-arch assembly, an internal and an external test package and `testdata/`; `internal/cgo` with a cgo preamble (`<stdlib.h>`, `"native.h"`) and `.c`/`.h` files; a local `replace` (`lib/`, its own `go.mod`); a fake module cache `modcache/` with an upper-case module path (`github.com/!upper/dep`) and an indirect dependency imported from inside the cache |
| `vendored/` | vendor mode (`go 1.22` plus `vendor/modules.txt`), a vendored package importing a package that is not vendored (unresolved) |
| `work/` | a `go.work` workspace: `app` imports a package of the `shared` workspace module |
| `nocache/` | a monorepo without a module cache, as in a fresh CI checkout (`crates/turborepo-trace/tests/affected_gomod.rs`): three modules with their own `go.mod` and `go.sum`, tests that import a third-party assertion library, a package used through a local `replace`, an import below two nested required modules, a package without third-party imports |
| `names/` | reads with a computed path, bounded by names (`crates/turborepo-trace/tests/affected_gonames.rs`): a loader that reads its parameter and a test that writes the file below `t.TempDir()`; a read-back below a temporary directory; a struct field joined with a parameter, given `"../fixtures"` by the test; a literal path from `runtime.Caller` returned by a helper |
| `xlang/<category>/` | cross-language parity scenarios, also run by turbotrace's runtime oracle: `assets` (`runtime.Caller` + `filepath.Dir/Join` in locals), `evaluator` (`fmt.Sprintf`, locals, constants), `loaders` (`plugin.Open`), `discovery` (`os.ReadDir`, embedded dir), `conditional` (build tags), `special` (`time.LoadLocation` without `time/tzdata`) |
