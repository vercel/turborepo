## Setup

Dependencies

1.  On OSX: `brew install sponge`
2.  Run `pnpm install` at root

Building

- Building turbo CLI: In `cli` run `make turbo`
- Using turbo to build turbo CLI: `./turbow.js`

Smoke Testing via examples:

1.  In `cli` run `make e2e`

## Debugging

1.  Install `go get dlv-dap`
2.  In VS Code Debugging tab, select `Basic Turbo Build` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example.

## Updating `turbo-install`

You might need to update `cli/npm/turbo-install` in order to support a new platform. When you do that you will need to link the module in order to be able to continue working. As an example, with `npm link`:

```sh
cd ~/repos/vercel/turborepo/cli/npm/turbo-install
npm link

# Run your build, e.g. `go build ./cmd/turbo` if you're on the platform you're adding.
cd ~/repos/vercel/turborepo/cli
go build ./cmd/turbo

# You can then run the basic example specifying the build asset path.
cd ~/repos/vercel/turborepo/examples/basic
TURBO_BINARY_PATH=~/repos/vercel/turborepo/cli/turbo.exe npm install
TURBO_BINARY_PATH=~/repos/vercel/turborepo/cli/turbo.exe npm link turbo
```

If you're using a different package manager replace npm accordingly.
