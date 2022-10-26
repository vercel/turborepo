# Turborepo

## Setup

Dependencies

1.  On OSX: `brew install sponge jq protobuf protoc-gen-go protoc-gen-go-grpc golang`
1.  Run `pnpm install` at root

Building

- Building `turbo` CLI: In `cli` run `make turbo`
- Using `turbo` to build `turbo` CLI: `./turbow.js`

## Testing

From the `cli/` directory, you can

- run smoke tests with `make e2e`
- run unit tests with `make test-go`

To run a single test, you can run `go test ./[path/to/package/]`. See more [in the Go docs](https://pkg.go.dev/cmd/go#hdr-Test_packages).

## Debugging

1. Install `go install github.com/go-delve/delve/cmd/dlv@latest`
2. In VS Code's "Run and Debug" tab, select `Build Basic` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example. This task is configured in [launch.json](./.vscode/launch.json).

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

## Publishing `turbo` to the npm registry

All builds are handled by manually triggering the appropriate [`release` GitHub workflow](./.github/workflows/release.yml).

To manually run a release:

1. `brew install goreleaser`
2. Add `GORELEASER_KEY` env var with the GoReleaser Pro key (ask @turbo-oss to get access to the key)
3. Update `version.txt` (do not commit this change to git manually)
4. `cd cli && make publish`

# Turbo Tooling

`turbo-tooling` uses a [Cargo workspaces][workspaces] monorepo. You'll find
several workspaces inside the `crates/` directory. In order to run a particular
crate, you can use the `cargo run -p [CRATE_NAME]` command.

## Testing

Install `cargo-nextest` (https://nexte.st/)

Run via:

```shell
cargo nextest run
```

For the test cases you need to run `yarn` to install some node_modules. See [Troubleshooting][] for solutions to common problems.

You can also create a little demo app and run

```shell
cargo run -p node-file-trace -- print demo/index.js
```

## Troubleshooting

See [Troubleshooting][].

[workspaces]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[troubleshooting]: troubleshooting.md
