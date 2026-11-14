# Js fixtures

Inputs for `crates/turborepo-trace-lang-js/tests/fixtures.rs` (snapshot tests) and corpus seeds for
`crates/turborepo-trace-lang-js/tests/conformance.rs`.

TODO: describe what each case in `basic/` exercises.

## `extract/`

Extraction-only inputs for `crates/turborepo-trace-lang-js/tests/extract_snapshots.rs` (one insta snapshot
per file, rendered through `Language::extract` so the dialect comes from the extension),
corpus seeds for the conformance suites, and part of the default tree of
`crates/turborepo-trace-lang-js/tests/oxc_diff.rs`.

| file | exercises |
|---|---|
| `esm.mjs` | every static ESM form, import attributes, dynamic imports (literal and computed), non-references (`import.meta`, property keys) |
| `cjs.cjs` | `require`, `require.resolve`, `module.require`, try/catch `OPTIONAL`, computed specifiers, shadowing and AMD negatives |
| `typescript.ts` | `import type`, inline `type`, `import x = require()`, triple-slash directives, TS generics, non-null `!`, type assertions, `typeof import()` |
| `declarations.d.ts` | everything in a declaration file is `TYPE_ONLY` |
| `component.tsx` / `component.jsx` | JSX text/attributes skipped, containers scanned, TS generic arrows vs JSX |
| `jsx_in_js.js` | the JSX heuristic for `.js` |
| `flow.js` | Flow generics: the JSX heuristic fails and the file is re-read without JSX |
| `tricky_lexing.js` | regex vs division, nested templates, blocks vs objects, keywords as properties |
| `minified.js` | minified-style code |
| `malformed.js` | unterminated constructs: one `ParseIncomplete`, extraction continues |
