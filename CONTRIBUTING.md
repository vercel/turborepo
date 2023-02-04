# Contributing to Turbo

Thanks for your interest in contributing to Turbo!

**Important note**: At the moment, Turbo is made up of two tools, Turborepo and Turbopack, built with different languages and toolchains. In the future, Turbo will become a single toolchain built on Rust and the Turbo engine. In the meantime, please follow the respective guide when contributing to each tool:

- [Contributing to Turbo](#contributing-to-turbo)
  - [Contributing to Turborepo](#contributing-to-turborepo)
    - [Building Turborepo](#building-turborepo)
    - [Running Turborepo Tests](#running-turborepo-tests)
      - [Go Tests](#go-tests)
      - [Rust Tests](#rust-tests)
  - [Debugging Turborepo](#debugging-turborepo)
  - [Benchmarking Turborepo](#benchmarking-turborepo)
  - [Updating `turbo`](#updating-turbo)
  - [Publishing `turbo` to the npm registry](#publishing-turbo-to-the-npm-registry)
  - [Contributing to Turbopack](#contributing-to-turbopack)
    - [Turbopack Architecture](#turbopack-architecture)
    - [Testing Turbopack](#testing-turbopack)
    - [Benchmarking Turbopack](#benchmarking-turbopack)
    - [Profiling Turbopack](#profiling-turbopack)
  - [Troubleshooting](#troubleshooting)

## Contributing to Turborepo

### Building Turborepo

Dependencies

1. Install `jq` and `sponge`

   On macOS: `brew install sponge jq`

1. Install [turborepo crate](https://github.com/vercel/turbo/blob/main/crates/turborepo/README.md) build requirements

1. Run `pnpm install` at root

Building

- Building `turbo` CLI: In `cli` run `make turbo`
- Using `turbo` to build `turbo` CLI: `./turbow.js`

### Running Turborepo Tests

#### Go Tests

From the root directory, you can

- run unit tests with `pnpm run --filter=cli test`
- run integration tests with `pnpm run --filter=cli integration-tests`
- run e2e tests with `pnpm run --filter=cli e2e`

To run a single Go test, you can run `go test ./[path/to/package/]`. See more [in the Go docs](https://pkg.go.dev/cmd/go#hdr-Test_packages).

#### Rust Tests

The recommended way to run tests is: `cargo nextest run -p turborepo-lib`.
You'll have to [install it first](https://nexte.st/book/pre-built-binaries.html).

You can also use the built in [`cargo test`](https://doc.rust-lang.org/cargo/commands/cargo-test.html) directly `cargo test -p turborepo-lib`.

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

See [the publishing guide](./release.md#release-turborepo).

## Contributing to Turbopack

Turbopack uses [Cargo workspaces][workspaces] in the Turbo monorepo. You'll find
several workspaces inside the `crates/` directory. In order to run a particular
crate, you can use the `cargo run -p [CRATE_NAME]` command. For example, to test the Next.js development server, run `cargo run -p next-dev`.

### Turbopack Architecture

A high-level introduction to Turbopack's architecture, workspace crates, and Turbo engine (the turbo-tasks crates) is available at [crates/turbopack/architecture.md](crates/turbopack/architecture.md).

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
