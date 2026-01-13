https://turborepo.dev/docs/guides/generating-code# `@turbo/gen`

Types for working with [Turborepo Generators](https://turborepo.dev/docs/guides/generating-code).

## Usage

Install:

```bash
pnpm add @turbo/gen --save-dev
```

Use types within your generator `config.ts`:

```ts filename="turbo/generators/config.ts"
import type { PlopTypes } from "@turbo/gen";

export default function generator(plop: PlopTypes.NodePlopAPI): void {
  // create a generator
  plop.setGenerator("Generator name", {
    description: "Generator description",
    // gather information from the user
    prompts: [
      ...
    ],
    // perform actions based on the prompts
    actions: [
      ...
    ],
  });
}
```

Learn more about Turborepo Generators in the [docs](https://turborepo.dev/docs/guides/generating-code)

---

For more information about Turborepo, visit [turborepo.dev](https://turborepo.dev) and follow us on X ([@turborepo](https://x.com/turborepo))!
