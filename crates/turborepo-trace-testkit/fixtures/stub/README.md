# Stub language fixtures

Inputs for the end-to-end tests in `crates/lang-stub/tests/e2e.rs` and for CLI demos with
`--features stub`. The stub syntax is documented in `crates/lang-stub/src/lib.rs`.

`basic/` has two entries:

- `main.stub` exercises extension and index probing, assets, an escaped string, a partial
  pattern (`"./locales/" + ?lang + ".json"`), a computed specifier, unresolved and optional
  references, a type-only reference (excluded by default), a build-phase reference, a bare
  package with a manifest, a directory read, a case mismatch and a cycle.
- `second.stub` shares `lib/util.stub` with `main.stub` and adds a symlink (`data/link.txt`)
  and an extensionless file detected by its `#!stub` shebang (`bin/tool`).

`unreferenced.txt` and `locales/README.md` must never appear in any output.
