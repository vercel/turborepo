# turbotrace graph interchange formats

`turborepo-trace-graph` reads and writes a whole-repo graph as versioned JSON. Both formats (JSON and
NDJSON) carry the same data as the [`GraphView`](src/view.rs) trait.

## JSON (`"schema": "turbotrace-graph"`, `"version": 1`)

One JSON object. Files are referenced by their index in `files`, and packages by their index
in `packages`. Paths are relative to the graph root, `/`-separated, with no leading `./`. A
directory path has no trailing slash, and the root directory is `""`.

```json
{
  "schema": "turbotrace-graph",
  "version": 1,
  "root": "/srv/repo",
  "packages": [
    {"name": ".", "version": "workspace", "root": ""},
    {"name": "lodash", "version": "4.17.21", "root": "node_modules/lodash"}
  ],
  "files": [
    {"path": "src/a.ts", "package": 0, "lang": "js"},
    {"path": "src/b.ts", "package": 0, "lang": "js"},
    {"path": "node_modules/lodash/index.js", "package": 1, "lang": "js", "external": true}
  ],
  "edges": [
    {"from": 0, "to": 1, "kind": "import", "line": 1, "specifier": "./b"},
    {"from": 0, "to": 1, "kind": "import", "flags": ["type_only"]},
    {"from": 1, "to": 2, "kind": "require", "specifier": "lodash"},
    {"from": 1, "kind": "require", "line": 9, "specifier": "./gone"}
  ],
  "uncertainties": [
    {"file": 1, "kind": "computed_specifier", "line": 12, "detail": "require(name)"}
  ],
  "probes": [
    {"file": 0, "path": "src/b.tsx"},
    {"file": 1, "path": "node_modules/left-pad"}
  ],
  "memberships": [
    {"file": 1, "dir": "src/locales", "pattern": "*.json"}
  ]
}
```

| Field | Type | Required | Meaning |
|---|---|---|---|
| `schema` | string | yes | Always `"turbotrace-graph"` |
| `version` | integer | yes | `1`. Readers reject other versions with a clear error |
| `root` | string | no | Absolute root the paths are relative to (informational) |
| `packages[]` | object | no (default `[]`) | `name`; `version` (`"workspace"` for first-party packages, otherwise the resolved version); `root` (default `""`) |
| `files[]` | object | no (default `[]`) | `path` (unique); `package` (index); `lang` (language id such as `js`, `py`, `go`, `rust`; omitted for non-source files); `external` (default `false`; true for third-party files and opaque package nodes) |
| `edges[]` | object | no | `from` (file index); `to` (file index, omitted or `null` when unresolved); `kind`; `phase` (default: the kind's default phase); `flags` (default `[]`); `line` (1-based, default `0` = unknown); `span` (`[start, end]` byte offsets, optional); `specifier` (optional) |
| `uncertainties[]` | object | no | `file`; `kind`; `line` (default `0`); `detail` (default `""`) |
| `probes[]` | object | no | `file`; `path`: a missing path that resolution of `file` looked for. Adding a file at `path`, or anywhere below it, can change how `file` resolves |
| `memberships[]` | object | no | `file`; `dir`; `pattern`: `file` depends on the listing of `dir` filtered by the glob `pattern`, which is relative to `dir`. `*`, `?` and `[..]` never cross `/`, `**` spans directories, and `{a,b}` is supported. Adding or deleting a matching file affects `file` |

Vocabulary: these are the stable snake_case names from `turborepo_trace_core::model`.

- `kind`: `import`, `require`, `dynamic_import`, `re_export`, `package`, `mod`, `embed`,
  `fs_read`, `fs_read_dir`, `worker`, `child_process`, `native_addon`, `shared_lib`,
  `manifest`, `asset`.
- Default phase: `build` for `embed`, `mod` and `package`; `runtime` for every other kind.
- `phase`: `runtime`, `build`.
- `flags`: `optional`, `conditional`, `lazy`, `type_only`.
- uncertainty `kind`: `computed_specifier`, `partial_specifier`, `pattern_too_broad`,
  `unresolved_specifier`, `aliased_loader`, `eval_like`, `env_dependent`, `cwd_dependent`,
  `conditional_resolution`, `generated_file`, `case_mismatch`, `parse_incomplete`,
  `unsupported_syntax`, `outside_base`.

Rules:

- Unknown top-level fields are rejected, so a typo doesn't silently drop data. Top-level
  arrays may appear in any order; references are checked once the whole document is read.
- Unknown fields inside records are ignored, which leaves room for additive extensions
  within version 1.
- Export omits default-valued fields. The writer emits each file's edges in this order:
  resolved edges first, then unresolved ones, each in insertion order.
- Export → import → export is byte-identical. A property test checks this.

## NDJSON (`json::write_ndjson`)

The same data as newline-delimited JSON, for streaming tools (`jq -c`, DuckDB `read_json`).
The first line is a header, `{"schema":"turbotrace-graph-ndjson","version":1,"root":"…"}`. Every
other line is one record with a `"type"` tag, in this order: `package`, `file`, `edge`,
`uncertainty`, `probe`, `membership`. The fields are those of the JSON format above, and
`package` and `file` records also carry their index as `"id"`:

```text
{"type":"package","id":0,"name":".","version":"workspace","root":""}
{"type":"file","id":0,"path":"src/a.ts","package":0,"lang":"js"}
{"type":"edge","from":0,"to":1,"kind":"import","line":1,"specifier":"./b"}
{"type":"probe","file":0,"path":"src/b.tsx"}
```
