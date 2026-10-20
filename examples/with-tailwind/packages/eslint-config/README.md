# `@repo/eslint-config`

Collection of internal ESLint flat configurations.

## TypeScript 7 note

TypeScript 7 is a native compiler and [does not ship a JavaScript API yet](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/#running-side-by-side-with-typescript-6.0), so tools like `typescript-eslint` cannot load it. Following the official side-by-side guidance, this package aliases its `typescript` dependency to the `@typescript/typescript6` compatibility package, which provides the TypeScript 6.0 API for linting. The apps and packages in this repository still compile and type-check with the native TypeScript 7 `tsc`.
