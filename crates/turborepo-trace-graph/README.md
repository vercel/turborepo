# turborepo-trace-graph

The query engine and language over the whole-repo graph of `turbo trace`.

- `GraphView`: the read-only graph interface. The composed shard store will implement it.
  `MemGraph` is the in-memory implementation, with a builder and import/export in the
  versioned JSON format (`SCHEMA.md`).
- `Engine`: evaluates queries.
  - Node sets are bitsets, and BFS buffers are pooled.
  - Indexes are built lazily: reverse CSR adjacency, a sorted probe index, and a
    directory → membership index.
- `output`: `list`, `json`, `dot` and `tree`.

## Language

```text
expr     := 'let' NAME '=' expr 'in' expr
          | primary (setop primary)*            -- left-associative, equal precedence
setop    := '+' | '-' | '^' | 'union' | 'except' | 'intersect'
primary  := '(' expr ')' | NAME '(' args ')' | WORD | "STRING" | '$' NAME
```

Literals (unquoted words or quoted strings):

| Literal | Meaning |
|---|---|
| `src/a.ts` | The file with this path. An unknown path is an error, with a hint |
| `src/**/*.ts`, `"src/*.{ts,tsx}"` | Glob over all paths (`*`, `?`, `[..]`, `{a,b}`, `**`) |
| `pkg:name`, `pkg:name@1.2.3`, `pkg:@scope/*` | Every file of the matching packages |

Rules for words:

- A word may contain `-`, but may not start with it. So `a - b` is a difference, and
  `my-file.ts` is one word.
- A comma inside `{}` belongs to the word.

Functions:

| Function | Result |
|---|---|
| `deps(x [, depth])` | `x` plus everything it transitively depends on |
| `rdeps(x [, depth])` | `x` plus everything that transitively depends on it (whole graph) |
| `somepath(a, b)` | One shortest path from `a` to `b` (BFS). The JSON output includes the ordered `path` |
| `allpaths(a, b)` | `deps(a) ^ rdeps(b)` |
| `kind(k, x)`, `phase(p, x)`, `flags(f, x)` | Nodes of `x` reached by an edge with that kind, phase or flag, whose source is also in `x`. Alternatives are separated with `\|`: `kind(import\|require, x)` |
| `lang(l, x)` | Nodes of `x` in language `l` (`js\|py` allowed) |
| `pkg(name, x)` | Nodes of `x` whose package matches (`name`, `name@version`, a glob, or `workspace` as the version) |
| `external(x)` | Third-party nodes of `x` |
| `uncertain(x)`, `uncertain(kind, x)` | Nodes of `x` with uncertainty (of that kind) |
| `affected(changes, entries [, affected\|unknown\|ignore])` | The affected entries (§17). The JSON output is the per-entry report |
| `modified(p, ...)`, `added(p, ...)`, `deleted(p, ...)` | Change sets, only valid inside `affected`. Arguments are raw paths, globs (not for `added`), `pkg:` (marks the package changed) or set expressions. Change sets combine with `+`, and a plain set counts as modified |

Traversal follows every resolved edge, including type-only edges. To leave them out
(`--runtime-only`), set `EngineOptions::traversal` to
`EdgeFilter::ALL.without_flags(RefFlags::TYPE_ONLY)`.

The host can bind variables with `Engine::bind`. For example, `$changes` from
`--since` lets you write `affected($changes, $tests)`.

## `affected`

Seeds (each one is recorded with its cause):

- modified or deleted nodes;
- nodes whose probes match an added path (the path itself or a missing ancestor directory);
- nodes whose directory memberships match an added or deleted path;
- nodes with a changed config input (`GraphView::inputs`: a manifest, tsconfig or framework config their resolution read);
- every node of a changed package, and for a package changed for some importers only (`ChangeSet::scoped`), the importers' files that link it;
- with `AffectedOptions::after_change` (the graph was built from the working tree, after the change, as the CLI does): nodes whose probes match a deleted path, since a file that resolved to it now probes it. The default treats the graph as the state before the change, where a deleted path is a node and no probe can name it.

Steps:

1. One multi-source reverse BFS from the seeds. It yields the affected entries and a
   shortest witness chain for each.
2. One reverse BFS from first-party nodes that carry counted uncertainty. Entries it
   reaches that are not already affected get the policy's verdict, with the nearest
   uncertain node and its reasons.
3. By default `partial_specifier` doesn't count, because globs are recorded as directory
   memberships.
