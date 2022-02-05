## Setup

Dependencies

1.  On OSX: `brew install sponge`
2.  Run `yarn` at root

Building

- Building turbo CLI: In `cli` run `make turbo`
- Using turbo to build turbo CLI: `./turbow.sh`

Smoke Testing via examples:

1.  In `cli` run `make e2e`

## Debugging

1.  Install `go get dlv-dap`
2.  In VS Code Debugging tab, select `Basic Turbo Build` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example.
