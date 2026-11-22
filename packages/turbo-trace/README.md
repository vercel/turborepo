# `@turbo/trace`

Plugins for jest and vitest that run only the test files [`turbo trace`](https://turborepo.dev/docs/reference/trace) selects. A repository adds one line to a runner configuration and writes no other code.

`turbo trace` is experimental, and so is this package.

## Usage

```bash
pnpm add @turbo/trace --save-dev
```

**Jest** (27 to 30): wrap the configuration.

```js
// jest.config.js
const { withTurboTrace } = require("@turbo/trace/jest");

module.exports = withTurboTrace({
  // The configuration you have.
});
```

**Vitest** (0.34 to 5) and vite-plus: add the plugin.

```ts
// vitest.config.ts
import { turboTrace } from "@turbo/trace/vitest";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [turboTrace()]
});
```

Then write the selection once, in a file named after its content, and name that file in the environment of the test runs:

```bash
turbo trace affected --for-runners > selection.json
export TURBO_TRACE_SELECTION="$PWD/selection-$(git hash-object selection.json).json"
mv selection.json "$TURBO_TRACE_SELECTION"
turbo run test
```

```jsonc
// turbo.json
{
  "tasks": {
    "test": {
      "env": ["TURBO_TRACE_SELECTION"]
    }
  }
}
```

Both parts matter for the cache of `turbo run`:

- [`env`](https://turborepo.dev/docs/reference/configuration#env) makes the value of the variable part of the task's cache key, so a filtered run and a full run do not share a result. Without it, a full run reuses the result of a filtered one.
- The value is a path, and the cache key does not see the content of the file. A selection that is rewritten at the same path would reuse the result of a run that skipped other tests. A name that holds the hash of the content changes whenever the selection does.

Add `selection*.json` to `.gitignore`.

Without `TURBO_TRACE_SELECTION`, both plugins do nothing: the configuration is returned as it is, and nothing is printed.

Use a `turbo` that has `turbo trace affected --for-runners`. It reads a configuration with these plugins as the configuration without them. An older `turbo` cannot read a wrapped jest configuration, and selects every test of its package.

## Which tests run

A plugin drops a test file only when the selection lists it in `unselected` and it is below none of the `runWhole` directories. Everything else runs:

- the selected files;
- files the selection does not know, such as a test file that was added after the selection was written;
- every file, when the selection says `runAll`;
- every file, when `runWhole` holds an empty string: a task of the root package starts tests that `turbo trace` does not map to files (a root script that runs mocha, for example), and the root package's directory holds every file;
- every file, with one warning line, when the selection cannot be read, is not a version 1 document, or belongs to another checkout.

The paths of the selection are relative to the repository root. The plugins take the outermost directory around the working directory that holds a file of the selection as that root.

Each call with a selection prints one line, `turbo trace: running 12 of 160 test files`, to stderr. With `TURBO_TRACE_REPORT=<file>`, it also appends one line of JSON to that file:

```json
{ "runner": "jest", "root": "packages/app", "ran": 12, "total": 160 }
```

`root` is the directory the runner works in, relative to the repository root (`.` is the root itself). For jest, `total` counts the files jest found and `ran` those that are left. For vitest, both count the files of the selection below `root`.

## Jest

- `withTurboTrace` takes an object, a function or a promise, and returns the same kind. For an object it returns a plain object at once, so other configurations can spread it: `{ ...require("../../jest.config.js"), rootDir: __dirname }`. The environment is read when the configuration is evaluated, which happens in each jest process.
- It sets [`filter`](https://jestjs.io/docs/configuration#filter-string) to an absolute path, and `passWithNoTests` and `forceExit`. With few test files left, jest runs them in its own process, and a test that leaves a handle open then keeps jest alive. All three are set only while a selection is active.
- Jest reads `filter` from the configuration it is started with. It ignores a `filter` inside an entry of `projects`. Wrap the configuration that has `projects`, not its entries. Jest also ignores a `filter` that comes from a `preset`.
- A configuration that already has a `filter` keeps it: the selection is not applied, and `withTurboTrace` prints one warning. To have both, call this package's filter from yours. `require("@turbo/trace/jest/filter")` is a function that takes the list of test paths and resolves to `{ filtered }`.
- A package whose configuration does not build on a wrapped one needs the line too. Without editing the configuration, pass the filter in the script: `jest --filter="$(node -p "require.resolve('@turbo/trace/jest/filter')")" --passWithNoTests`.
- `--shard` splits the files that are left. `--testPathPattern`, paths, `--findRelatedTests`, `--listTests` and watch mode see the filtered list. `--skipFilter` turns the plugin off.
- A coverage threshold counts only the tests that ran, so it can fail in a filtered run. Check coverage in full runs.

## Vitest

- The plugin adds the dropped files of the project to [`test.exclude`](https://vitest.dev/config/#exclude), after your excludes or vitest's defaults, and sets `passWithNoTests`. `--shard`, file filters, `related` and watch mode work on the files that are left.
- It applies to the configuration it is in. With `projects` or a workspace file, add it to each project's configuration, or to a configuration they extend.
- Vitest reads `passWithNoTests` from the root configuration only. When the plugin is only in the projects and the selection drops every test file, vitest 3.1 and later still exit with code 0: the plugin sets the option for the run. Before 3.1 (a workspace file), pass `--passWithNoTests`.
- It does nothing outside vitest: a `vite.config.ts` that `vite build` or a linter also loads is not changed.
- Typechecking (`typecheck.include`) is not filtered. A coverage threshold can fail in a filtered run, as with jest.

## Where to declare it

Declare `@turbo/trace` in the `package.json` of each package whose configuration imports it. Declared only at the repository root, it works with a hoisting package manager, but tools that check for unlisted dependencies will report it.
