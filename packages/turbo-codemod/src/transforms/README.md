# `@turbo/codemod` Transformers

## Adding new transformers

Add new transformers using the [plopjs](https://github.com/plopjs/plop) template by running:

```bash
pnpm add-transformer
```

New Transformers will be automatically surfaced to the `transform` CLI command and used by the `migrate` CLI command when appropriate.

## How it works

Transformers are loaded automatically from the `src/transforms/` directory via the [`loadTransforms`](../utils/loadTransformers.ts) function.

All new transformers must contain a default export that matches the [`Transformer`](../types.ts) type:

```ts
export type Transformer = {
  name: string;
  value: string;
  introducedIn: string;
  transformer: (args: TransformerArgs) => TransformerResults;
};
```

## Writing a Transform

Transforms are ran using the [TransformRunner](../runner/Runner.ts). This class is designed to make writing transforms as simple as possible by abstracting away all of the boilerplate that determines what should be logged, saved, or output as a result.

To use the TransformRunner:

1. Transform each file in memory (do not write it back to disk `TransformRunner` takes care of this depending on the options passed in by the user), and pass to `TransformRunner.modifyFile` method.
2. If the transform encounters an unrecoverable error, pass it to the `TransformRunner.abortTransform` method.
3. When all files have been modified and passed to `TransformRunner.modifyFile`, call `TransformRunner.finish` method to write the files to disk (when not running in `dry` mode) and log the results.
