# Contributing to Turbo

Thanks for your interest in contributing to Turbo!

**Important note**: At the moment, Turbo is made up of two tools, Turborepo and Turbopack, built with different languages and toolchains. In the future, Turbo will become a single toolchain built on Rust and the Turbo engine. In the meantime, please follow the respective guide when contributing to each tool:

- [Contributing to Turbo](#contributing-to-turbo)
  - [Contributing to Turborepo](#contributing-to-turborepo)
    - [Building Turborepo](#building-turborepo)
    - [Testing Turborepo](#testing-turborepo)
  - [Debugging Turborepo](#debugging-turborepo)
  - [Benchmarking Turborepo](#benchmarking-turborepo)
  - [Updating `turbo`](#updating-turbo)
  - [Publishing `turbo` to the npm registry](#publishing-turbo-to-the-npm-registry)
  - [Contributing to Turbopack](#contributing-to-turbopack)
    - [Testing Turbopack](#testing-turbopack)
    - [Benchmarking Turbopack](#benchmarking-turbopack)
  - [Troubleshooting](#troubleshooting)

## Contributing to Turborepo

### Building Turborepo

Dependencies

1. Install `jq` and `sponge`

   On macOS: `brew install sponge jq`

1. Install [turbo/shim](https://github.com/vercel/turbo/blob/main/shim/README.md) build requirements

1. Run `pnpm install` at root

Building

- Building `turbo` CLI: In `cli` run `make turbo`
- Using `turbo` to build `turbo` CLI: `./turbow.js`

### Testing Turborepo

From the `cli/` directory, you can

- run smoke tests with `make e2e`
- run unit tests with `make test-go`

To run a single test, you can run `go test ./[path/to/package/]`. See more [in the Go docs](https://pkg.go.dev/cmd/go#hdr-Test_packages).

## Debugging Turborepo

1. Install `go install github.com/go-delve/delve/cmd/dlv@latest`
1. In VS Code's "Run and Debug" tab, select `Build Basic` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example. This task is configured in [launch.json](./.vscode/launch.json).

## Benchmarking Turborepo

1. Build Turborepo [as described above](#Setup)
1. From the `benchmark/` directory, run `pnpm run benchmark`.

## Updating `turbo`

You might need to update `packages/turbo` in order to support a new platform. When you do that you will need to link the module in order to be able to continue working. As an example, with `npm link`:

```sh
cd ~/repos/vercel/turbo/packages/turbo
npm link

# Run your build, e.g. `go build ./cmd/turbo` if you're on the platform you're adding.
cd ~/repos/vercel/turbo/cli
go build ./cmd/turbo

# You can then run the basic example specifying the build asset path.
cd ~/repos/vercel/turbo/examples/basic
TURBO_BINARY_PATH=~/repos/vercel/turbo/cli/turbo.exe npm install
TURBO_BINARY_PATH=~/repos/vercel/turbo/cli/turbo.exe npm link turbo
```

If you're using a different package manager replace npm accordingly.

## Publishing `turbo` to the npm registry

All builds are handled by manually triggering the appropriate [`release` GitHub workflow](./.github/workflows/release.yml).

To manually run a release:

1. `brew install goreleaser`
2. Add `GORELEASER_KEY` env var with the GoReleaser Pro key (ask @turbo-oss to get access to the key)
3. Update `version.txt` (do not commit this change to git manually)
4. `cd cli && make publish`

## Contributing to Turbopack

Turbopack uses [Cargo workspaces][workspaces] in the Turbo monorepo. You'll find
several workspaces inside the `crates/` directory. In order to run a particular
crate, you can use the `cargo run -p [CRATE_NAME]` command.

### Testing Turbopack

Install `cargo-nextest` (https://nexte.st/):

`cargo install cargo-nextest`

Run via:

```shell
cargo nextest run
```

For the test cases you need to run `pnpm install` to install some node_modules. See [Troubleshooting][] for solutions to common problems.

You can also create a little demo app and run

```shell
cargo run -p node-file-trace -- print demo/index.js
```

### Benchmarking Turbopack

See [the benchmarking README for Turbopack](crates/next-dev/benches/README.md) for details.

### Profiling Turbopack

See [the profiling docs for Turbopack](https://turbo.build/pack/docs/advanced/profiling) for details.

## Troubleshooting

See [Troubleshooting][].

[workspaces]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[troubleshooting]: troubleshooting.md
