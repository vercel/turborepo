# data-inputs

A workspace whose packages are consumed through an unbuilt `dist/`, in the shape of a large
internal monorepo: which of a build task's inputs lend their open uncertainty to the tests
that load the task's outputs.

| Package | Producers of `dist/` | The uncertain file | The consumer's test |
|---|---|---|---|
| `packages/assets` | `build`: `tsc -p tsconfig.build.json`; `type-check`: `tsc -b` (declarations only) with turbo `inputs: ["$TURBO_DEFAULT$"]` and the declared output `dist` | `scripts/visual/extract.mjs` (the entry of another script; a member of the turbo inputs), `src/names.test.ts` (in the type-checked project, excluded from the build) and `tools/upload.ts` (named by the type-checked project's `include`; the entry of another script) | `apps/web/lib/icon.test.ts`: not on the floor |
| `packages/gen` | `build`: `node scripts/build.mjs` | `scripts/build.mjs`: the task runs it | `apps/web/lib/gen.test.ts`: on the floor |
| `packages/lib` | `build`: `tsc -p tsconfig.json` (emits JavaScript) | `src/index.ts`: the consumer runs its emitted copy | `apps/web/lib/lib.test.ts`: on the floor |
| `packages/tool` | `build`: `frobnicate src --out dist` (an unknown command) | `src/index.ts`: an unknown command may compile, bundle or copy the files of its package into its output, or run them | `apps/web/lib/tool.test.ts`: on the floor |
| `packages/analytics` | `build`: `tsc --build --force` (emits JavaScript), turbo `dependsOn: ["^build"]`; the package depends on `@acme/gen` without importing it | `packages/gen/scripts/build.mjs`, run by the upstream `build` | `apps/web/lib/analytics.test.ts`: not on the floor (other packages' tasks don't write into this one) |
| `packages/typed` | `build`: `pnpm typegen && tsc -b` | `scripts/typegen.mjs`, run by the package's own `typegen`, which may generate the project's sources | `apps/web/lib/typed.test.ts`: on the floor |
