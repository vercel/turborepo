# with-mcp-servers

This is a community-maintained example. If you experience a problem, please submit a pull request with a fix. GitHub Issues will be closed.

A Turborepo monorepo demonstrating how to structure multiple [Model Context Protocol (MCP)](https://modelcontextprotocol.io) servers as isolated workspace packages, with Turbo's build pipeline ensuring correct dependency ordering.

## Using this example

Run the following command:

```sh
npx create-turbo@latest --example with-mcp-servers
```

## What's inside?

This monorepo includes the following packages and apps:

### Apps

- `mcp-client`: A Node.js CLI that connects to both MCP servers via stdio transport and runs demo tool calls

### Packages

- `@repo/mcp-calculator`: An MCP server exposing four arithmetic tools — `add`, `subtract`, `multiply`, `divide`
- `@repo/mcp-file-reader`: An MCP server exposing two read-only filesystem tools — `read_file`, `list_directory`. It takes an allowed root directory as its first CLI argument and rejects any path that escapes it (including through symlinks), since MCP tool arguments are untrusted input.
- `@repo/eslint-config`: Shared ESLint configuration (includes `eslint` and `typescript-eslint`)
- `@repo/typescript-config`: Shared `tsconfig.json` bases used throughout the monorepo

Each MCP server is a standalone Node.js ESM package built with `tsc` and uses stdio transport from the official [`@modelcontextprotocol/sdk`](https://github.com/modelcontextprotocol/typescript-sdk). Each one exposes a side-effect-free `createServer()` factory (used by the tests) and a separate executable entry (used by the client), published through the package's `exports` field.

## Build

To build all apps and packages, run the following command:

```sh
pnpm build
```

Turbo's `^build` dependency ensures the server packages compile before the client app.

## Run the demo

```sh
pnpm start
```

This runs the `start` task through Turbo, which builds anything that is out of date first, then launches the client. The client resolves each server's compiled executable through the package manager, spawns both as child processes, and runs a few tool calls.

## Test

Both server packages have integration tests that exercise their tools over an in-memory MCP transport:

```sh
pnpm test
```

## Develop

To watch all packages in parallel, run the following command:

```sh
pnpm dev
```

This starts `tsc --watch` in every package — it recompiles on change but doesn't run anything. Re-run `pnpm start` in another terminal to exercise your changes.

## Useful Links

- [MCP TypeScript SDK](https://github.com/modelcontextprotocol/typescript-sdk)
- [Turborepo docs](https://turborepo.dev/docs)
- [Remote Caching](https://turborepo.dev/docs/core-concepts/remote-caching)
