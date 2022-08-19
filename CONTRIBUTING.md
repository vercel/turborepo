## Setup

Dependencies

1.  On OSX: `brew install sponge jq protobuf protoc-gen-go protoc-gen-go-grpc golang`
1.  Run `pnpm install` at root

Building

- Building turbo CLI: In `cli` run `make turbo`
- Using turbo to build turbo CLI: `./turbow.js`

## Testing

From the `cli/` directory, you can

- run smoke tests with `make e2e`
- run unit tests with `make test-go`

To run a single test, you can run `go test ./[path/to/package/]`. See more [in the Go docs](https://pkg.go.dev/cmd/go#hdr-Test_packages).

## Debugging

1.  Install `go get dlv-dap`
1.  In VS Code Debugging tab, select `Basic Turbo Build` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example.

## Updating `turbo`

You might need to update `packages/turbo` in order to support a new platform. When you do that you will need to link the module in order to be able to continue working. As an example, with `npm link`:

```sh
cd ~/repos/vercel/turborepo/packages/turbo
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
