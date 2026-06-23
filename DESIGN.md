# Structured Task Inputs

This document describes a proposed structured shape for `turbo.json` task
`inputs`. The goal is to avoid encoding input behavior in sentinel glob strings
and provide a typed shape that can express when and where inputs are resolved.

## Motivation

Today, task inputs are configured as strings:

```json
{
  "tasks": {
    "build": {
      "inputs": ["$TURBO_DEFAULT$", "src/**"]
    }
  }
}
```

That syntax is compact, but special behavior is encoded inside glob strings:

- `$TURBO_DEFAULT$` includes default package inputs.
- `$TURBO_EXTENDS$` extends inherited package configuration arrays instead of
  replacing them.

As input behavior becomes more expressive, the string syntax becomes less
glob-like and harder to validate, document, and extend.

## Proposed Shape

`inputs` accepts a mixed array of legacy strings and structured input config
objects:

```ts
type Inputs = Array<string | StartupInput | JitInput | DependencyOutputsInput>;

interface StartupInput {
  mode: "startup";
  globs?: string[];
  withDefaults?: boolean;
}

interface JitInput {
  mode: "jit";
  globs?: string[];
  withDefaults?: boolean;
}

interface DependencyOutputsInput {
  mode: "dependencyOutputs";
  from?: string[];
  globs?: string[];
}
```

Defaults:

- `startup.withDefaults` defaults to `false`.
- `startup.globs` defaults to `[]`.
- `jit.withDefaults` defaults to `false`.
- `jit.globs` defaults to `[]`.
- `dependencyOutputs.from` defaults to all direct task dependencies.
- `dependencyOutputs.globs` defaults to all declared outputs from selected
  dependency tasks.
- `from` is only valid for `mode: "dependencyOutputs"`.
- `withDefaults` is only valid for `mode: "startup"` or `mode: "jit"`.
- Structured object entries must specify `mode`.

Each mode may appear at most once per task definition after normalization.
Legacy string entries are collected into a single normalized `startup` entry.
Modes are semantically separate; globs from one mode do not include or exclude
files from another mode.

## Normalization

Legacy string entries normalize into the `startup` mode.

```json
{
  "inputs": ["abc"]
}
```

Normalizes to:

```ts
{
  mode: "startup",
  globs: ["abc"],
  withDefaults: false
}
```

`$TURBO_DEFAULT$` sets `withDefaults: true` on the normalized startup entry.

```json
{
  "inputs": ["$TURBO_DEFAULT$", "def"]
}
```

Normalizes to:

```ts
{
  mode: "startup",
  globs: ["def"],
  withDefaults: true
}
```

Multiple entries for the same mode are equivalent to one merged entry internally,
but public config should reject duplicate structured modes to avoid ambiguous
intent. Legacy string entries normalize to one `startup` entry, so they cannot be
combined with a structured `startup` entry.

`$TURBO_EXTENDS$` is not an input mode sentinel. It is an array inheritance marker
used by package configurations and must be resolved before structured input
normalization. After package configuration inheritance is resolved, the merged
`inputs` array is normalized as though the user had written the merged array
directly.

For example, this root configuration:

```json
{
  "tasks": {
    "build": {
      "inputs": ["$TURBO_DEFAULT$", "src/**"]
    }
  }
}
```

And this package configuration:

```json
{
  "extends": ["//"],
  "tasks": {
    "build": {
      "inputs": [
        "$TURBO_EXTENDS$",
        {
          "mode": "jit",
          "globs": ["src/generated/**"]
        }
      ]
    }
  }
}
```

Resolve to the same normalized inputs as:

```json
{
  "inputs": [
    "$TURBO_DEFAULT$",
    "src/**",
    {
      "mode": "jit",
      "globs": ["src/generated/**"]
    }
  ]
}
```

Then normalize to:

```ts
[
  {
    mode: "startup",
    globs: ["src/**"],
    withDefaults: true
  },
  {
    mode: "jit",
    globs: ["src/generated/**"],
    withDefaults: false
  }
]
```

If `$TURBO_EXTENDS$` causes inherited and local entries to produce duplicate
structured modes after normalization, the same duplicate-mode validation applies.
For example, extending inherited legacy string inputs and then adding a structured
`startup` entry is invalid because the inherited strings normalize to `startup`.

This is invalid:

```json
{
  "inputs": [
    "$TURBO_DEFAULT$",
    {
      "mode": "startup",
      "globs": ["src/**"]
    }
  ]
}
```

The error should explain that legacy string inputs normalize to `startup` and ask
the user to use either legacy startup inputs or one structured `startup` entry.

## Modes

### `startup`

`startup` inputs are resolved before task execution starts. This is the default
mode and matches normal task input hashing.

`startup.globs` are resolved relative to the consuming task's package root and
use the same include, exclude, ordering, and negation behavior as legacy task
input strings.

```json
{
  "inputs": [
    {
      "mode": "startup",
      "withDefaults": true,
      "globs": ["!src/generated/**"]
    }
  ]
}
```

### `jit`

`jit` inputs are resolved after this task's dependencies complete and before the
task itself runs. This supports inputs that are created by dependency tasks.

`jit.globs` are resolved relative to the consuming task's package root and use
the same include, exclude, ordering, and negation behavior as legacy task input
strings.

`jit.withDefaults` includes default package inputs in the deferred input set.
This matches `startup.withDefaults`, but resolves after this task's dependencies
complete.

```json
{
  "inputs": [
    {
      "mode": "jit",
      "globs": ["src/generated/**"]
    }
  ]
}
```

```json
{
  "inputs": [
    {
      "mode": "jit",
      "withDefaults": true,
      "globs": ["!src/generated/**"]
    }
  ]
}
```

In dry runs, a task using `jit` inputs reports a deferred hash because the final
input set is not available until execution time.

### `dependencyOutputs`

`dependencyOutputs` inputs are resolved after this task's dependencies complete
and before this task runs. They hash materialized declared outputs of dependency
tasks that already exist in the expanded task graph.

```json
{
  "tasks": {
    "codegen": {
      "outputs": ["src/generated/**"]
    },
    "check-types": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!src/generated/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"]
        }
      ]
    }
  }
}
```

Semantics:

- By default, only direct task dependencies are considered.
- Dependency edges come from `dependsOn`; `inputs` does not create task graph
  edges.
- Only files matched by the dependency task's declared `outputs` are eligible.
- A dependency task with no declared `outputs` is invalid when selected by
  `dependencyOutputs`.
- Missing output directories and empty glob matches are valid and silent.
- Omitted `globs` means all declared outputs from the selected dependency tasks.
- `globs` filters the selected dependency tasks' declared outputs, but only files
  inside each dependency task's declared `outputs` are eligible.

For `dependencyOutputs`, `globs` are evaluated relative to each producer task's
package root, matching the producer's `outputs` semantics. Internally, matched
files can be normalized to workspace-relative paths for hashing and summaries.

This mirrors legacy glob behavior, but with a different universe of eligible
files. `startup` and `jit` select package files from the consuming task's package
root. `dependencyOutputs` selects files from the declared outputs of selected
dependency tasks.

This hashes all declared outputs from `codegen`:

```json
{
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "from": ["codegen"]
    }
  ]
}
```

This hashes selected declared outputs from `codegen`:

```json
{
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "from": ["codegen"],
      "globs": ["src/generated/**", "!src/generated/**/*.map"]
    }
  ]
}
```

In dry runs, a task using `dependencyOutputs` reports:

```txt
Deferred because dependencyOutputs mode was used.
```

## Selecting Dependency Tasks

By default, `dependencyOutputs` reads declared outputs from all direct task
dependencies.

```json
{
  "dependsOn": ["^build"],
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "globs": ["dist/**", "!dist/**/*.map"]
    }
  ]
}
```

This includes matching declared outputs from the direct dependency tasks created
by `^build`. The default selection does not include transitive task
dependencies.

To select a subset of dependency tasks, use `from`:

```json
{
  "dependsOn": ["^build", "codegen"],
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "from": ["^build"]
    }
  ]
}
```

`from` accepts the same task id grammar as `dependsOn`, but it never creates task
graph edges. It resolves against already-expanded dependency task nodes in the
current task's dependency subgraph. If an entry in `from` does not match any
eligible dependency task node, configuration validation should fail and tell the
user to add the task to `dependsOn` or remove it from `inputs`.

Task ids in `from` resolve the same way they do in `dependsOn`. For example,
`"build"` refers to the current package's `build` task, `"pkg#build"` refers to
that exact package task, and `"^build"` refers to `build` tasks in dependency
packages. These selectors only match nodes that already exist in the expanded
task graph.

A direct task dependency is a task node with an incoming edge into the current
task node in the expanded task graph. Explicit `from` entries may also select
transitive dependency task nodes when the selector resolves to nodes that already
exist in the current task's dependency subgraph. `dependencyOutputs` reads from
the expanded task graph and never creates new graph edges.

Package-qualified task ids can select a specific dependency task:

```json
{
  "dependsOn": ["^codegen"],
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "from": ["pkg#codegen"]
    }
  ]
}
```

Package-qualified task ids are valid when they match expanded dependency task
nodes. The package-qualified task id does not need to appear literally in
`dependsOn`.

For example, if `pkg#build` is created by expanding `^build`, the consumer can
select only that producer's outputs:

```json
{
  "dependsOn": ["^build"],
  "inputs": [
    {
      "mode": "dependencyOutputs",
      "from": ["pkg#build"]
    }
  ]
}
```

## Examples

Exclude generated files from startup inputs, then hash dependency outputs
after dependencies complete:

```json
{
  "tasks": {
    "codegen": {
      "outputs": ["src/generated/**"]
    },
    "check-types": {
      "dependsOn": ["codegen"],
      "inputs": [
        {
          "mode": "startup",
          "withDefaults": true,
          "globs": ["!src/generated/**"]
        },
        {
          "mode": "dependencyOutputs",
          "from": ["codegen"],
          "globs": ["src/generated/**"]
        }
      ]
    }
  }
}
```

Mix legacy startup strings with structured JIT inputs:

```json
{
  "inputs": [
    "$TURBO_DEFAULT$",
    "!src/generated/**",
    {
      "mode": "jit",
      "globs": ["src/generated/**"]
    }
  ]
}
```

## Validation

Recommended validation rules:

- Reject unknown modes.
- Reject structured object entries without `mode`.
- Reject duplicate modes after legacy string normalization.
- Reject legacy startup strings combined with a structured `startup` entry.
- Reject `from` unless `mode` is `"dependencyOutputs"`.
- Reject `withDefaults` unless `mode` is `"startup"` or `"jit"`.
- Reject `dependencyOutputs` when the task has no dependency tasks to select.
- Reject `dependencyOutputs.from` entries that do not match eligible dependency
  task nodes in the expanded task graph.
- Reject selected dependency tasks with no declared `outputs`.
- Reject negative-only `startup` or `jit` `globs` unless `withDefaults` is true.
- Reject malformed globs.
- Reject sentinel strings inside structured `globs`.
- Reject `$TURBO_EXTENDS$` during input normalization if package configuration
  inheritance did not already consume it.
- Do not warn or error for missing files, missing output directories, empty
  output matches, or empty declared output globs.

Example error for duplicate startup inputs:

```txt
Invalid inputs for task "build".

Legacy input strings normalize to mode "startup", but this task also declares a
structured "startup" input.

Use either legacy startup inputs:

  "inputs": ["$TURBO_DEFAULT$", "src/**"]

Or one structured startup input:

  "inputs": [
    {
      "mode": "startup",
      "withDefaults": true,
      "globs": ["src/**"]
    }
  ]
```

Example error for `dependencyOutputs` without a matching dependency:

```txt
Invalid inputs for task "check-types".

"dependencyOutputs.from" contains "codegen", but "codegen" does not match any
eligible dependency task node for this task.

Add it to dependsOn or remove it from dependencyOutputs.from.
```

Example error for selected dependency tasks with no declared outputs:

```txt
Invalid inputs for task "check-types".

Selected dependency task "codegen" does not declare outputs, so it cannot
contribute dependency output inputs.

Add outputs to "codegen" or remove it from dependencyOutputs.from.
```

## Compatibility

Legacy string arrays remain valid. Sentinel strings can continue to normalize
into the structured representation internally. New behavior should prefer object
syntax instead of adding more sentinel strings.

Package configuration array extension remains valid for `inputs` via
`$TURBO_EXTENDS$`. The marker is consumed during package configuration resolution;
it should not appear in the structured representation and should not be accepted
inside structured `globs`.
