---
title: "@turbo/gen"
description: Quickly generate new code in your Turborepo.
product: turborepo
type: reference
summary: Reference for the `@turbo/gen` package that provides type definitions for Turborepo code generators.
related:
  - /docs/reference/generate
  - /docs/guides/generating-code
---

Use this package for type definitions in your [Turborepo code generators](/docs/reference/generate).

```ts title="./turbo/generators/my-generator.ts"
import type { PlopTypes } from "@turbo/gen"; // [!code highlight]

// [!code word:PlopTypes]
export default function generator(plop: PlopTypes.NodePlopAPI): void {
  // Create a generator
  plop.setGenerator("Generator name", {
    description: "Generator description",
    // Gather information from the user
    prompts: [
      ...
    ],
    // Perform actions based on the prompts
    actions: [
      ...
    ],
  });
}
```

## Turborepo variables

When running generator actions, Turborepo injects a `turbo` object into the action answers data. This object provides information about the repository and the generator's context.

### `turbo.paths`

| Variable | Type | Description |
| --- | --- | --- |
| `turbo.paths.cwd` | `string` | The current working directory when the generator was invoked. |
| `turbo.paths.root` | `string` | The root directory of the Turborepo project. |
| `turbo.paths.workspace` | `string \| undefined` | The root directory of the workspace that contains the generator. `undefined` for root-level generators that are not within a workspace. |

### `turbo.configs`

An array of `TurboConfig` objects representing all `turbo.json` and `turbo.jsonc` files found in the repository.

Each `TurboConfig` object has the following shape:

| Property | Type | Description |
| --- | --- | --- |
| `config` | `object` | The parsed contents of the Turbo configuration file. |
| `turboConfigPath` | `string` | The absolute path to the Turbo configuration file. |
| `workspacePath` | `string` | The absolute path to the workspace directory containing the configuration. |
| `isRootConfig` | `boolean` | Whether the configuration is from the root of the monorepo. |

For more information, [visit the Generating code guide](/docs/guides/generating-code).
