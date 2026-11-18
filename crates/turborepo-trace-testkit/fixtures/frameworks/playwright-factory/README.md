# playwright-factory

A pnpm-style workspace whose Playwright configs come from a factory in a workspace package:
`defineConfig(__dirname)` of `@acme/browser-kit/preset` (`packages/browser-kit`, linked into each
app's `node_modules`; its `exports` point at a `build/` that a checkout does not have, and the
`types` condition at the TypeScript source). All names are made up.

- `apps/docs`: the config is the factory's result. Specs: `app/*.e2e.ts`, `app/*.e2e.chromium.ts`.
  `lib/format.test.ts` is a vitest test, not a spec.
- `apps/shop`: spreads the result and overrides `testDir` and `testMatch`. Spec: `spec/cart.spec.ts`.
- `apps/blog`: `.cts` config that `require`s the factory.
- `apps/opaque`: a factory (`@acme/browser-kit/opaque`) whose `testMatch` is a call into a
  third-party package: unknown, reported, default pattern.
- `apps/plain`: `defineConfig` of the installed `@playwright/test` (the identity).
- `apps/legacy`: a jest config that passes its object through the workspace package's
  `defineProject` (jest and vitest do not follow factories: the identity, as before).
