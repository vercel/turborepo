## Quick start

Install dependencies from the workspace root, then start Gatsby:

```sh
pnpm --filter web dev
```

Open [http://localhost:8000](http://localhost:8000) and edit `src/pages/index.tsx` to see updates.

The app consumes the shared `@repo/ui` TypeScript source through `gatsby-plugin-compile-es6-packages`.

## Production

From the workspace root:

```sh
pnpm --filter web build
pnpm --filter web start
```

The Gatsby production preview is available at [http://localhost:9000](http://localhost:9000).
