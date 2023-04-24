# Contributing to Turbo

Thanks for your interest in contributing to Turbo!

**Important note**: At the moment, Turbo is made up of two tools, Turborepo and Turbopack, built with different languages and toolchains. In the future, Turbo will become a single toolchain built on Rust and the Turbo engine. In the meantime, please follow the respective guide when contributing to each tool:

- [Contributing to Turbo](#contributing-to-turbo)
  - [Contributing to Turborepo](#contributing-to-turborepo)
    - [Building Turborepo](#building-turborepo)
    - [TLS Implementation](#tls-implementation)
    - [Running Turborepo Tests](#running-turborepo-tests)
      - [Go Tests](#go-tests)
  - [Debugging Turborepo](#debugging-turborepo)
  - [Benchmarking Turborepo](#benchmarking-turborepo)
  - [Updating `turbo`](#updating-turbo)
  - [Manually testing `turbo`](#manually-testing-turbo)
  - [Publishing `turbo` to the npm registry](#publishing-turbo-to-the-npm-registry)
  - [Adding A New Crate](#adding-a-new-crate)
  - [Contributing to Turbopack](#contributing-to-turbopack)
    - [Turbopack Architecture](#turbopack-architecture)
    - [Testing Turbopack](#testing-turbopack)
    - [Benchmarking Turbopack](#benchmarking-turbopack)
    - [Profiling Turbopack](#profiling-turbopack)
  - [Troubleshooting](#troubleshooting)

## Contributing to Turborepo

### Building Turborepo

Dependencies

1. Install [turborepo crate](./crates/turborepo/README.md) build requirements

1. Run `pnpm install` at root

Building

- Building `turbo` CLI: In `cli` run `make turbo`
- Using `turbo` to build `turbo` CLI: `./turbow.js`

### TLS Implementation

Turborepo uses `reqwest`, a Rust HTTP client, to make requests to the Turbo API. `reqwest` supports two TLS
implementations: `rustls` and `native-tls`. `rustls` is a pure Rust implementation of TLS, while `native-tls`
is a wrapper around OpenSSL. Turborepo allows users to select which implementation they want with the `native-tls`
and `rustls-tls` features. By default, the `native-tls` feature is selected---this is done so that `cargo build` works
out of the box. If you wish to select `rustls-tls`, you may do so by passing `--no-default-features --features rustls-tls`
to the build command. This allows for us to build for more platforms, as `native-tls` is not supported everywhere.

### Running Turborepo Tests

Install dependencies

On macOS:

```bash
brew install moreutils jq zstd # (moreutils is for sponge)
```

#### Go Tests

First: `npm install -g turbo`.

Then from the root directory, you can run:

- Go unit tests
  ```bash
  pnpm test -- --filter=cli
  ```
- A single Go unit test (see more [in the Go docs](https://pkg.go.dev/cmd/go#hdr-Test_packages))
  ```bash
  cd cli && go test ./[path/to/package/]
  ```
- Rust unit tests ([install `nextest` first](https://nexte.st/book/pre-built-binaries.html))
  ```bash
  cargo nextest run -p turborepo-lib --features rustls-tls
  ```
  You can also use the built in [`cargo test`](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
  directly with `cargo test -p turborepo-lib`.
- CLI Integration tests
  ```bash
  pnpm test -- --filter=turborepo-tests-integration
  ```
- E2E test
  ```bash
  pnpm -- turbo e2e --filter=cli
  ```
- Example tests
  ```bash
  pnpm test -- --filter=turborepo-tests-examples -- <example-name> <packagemanager>
  ```

## Debugging Turborepo

1. Install `go install github.com/go-delve/delve/cmd/dlv@latest`
1. In VS Code's "Run and Debug" tab, select `Build Basic` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example. This task is configured in [launch.json](./.vscode/launch.json).

## Benchmarking Turborepo

1. `make turbo-prod`
2. From the `benchmark/` directory, run `pnpm run benchmark`.

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

## Manually testing `turbo`

Before releasing, it's recommended to test the `turbo` binary manually.
Here's a checklist of testing strategies to cover:

- Test `login`, `logout`, `login --sso-team`, `link`, `unlink`
- Test `prune` (Note `turbo` here is the unreleased turbo binary)
  - `npx create-turbo --use-pnpm prune-test && cd prune-test`
  - `turbo --skip-infer prune --scope=docs && cd out && pnpm install --frozen-lockfile`
  - `turbo --skip-infer build`
- Test `--dry-run` and `--graph`.
- Test with and without daemon.

There are also multiple installation scenarios worth testing:

- Global-only. `turbo` is installed as global binary, no local `turbo` in repository.
- Local-only. `turbo` is installed as local binary, no global `turbo` in PATH. turbo` is invoked via a root package script.
- Global + local. `turbo` is installed as global binary, and local `turbo` in repository. Global `turbo` delegates to local `turbo`

Here are a few repositories that you can test on:

- [next.js](https://github.com/vercel/next.js)
- [tldraw](https://github.com/tldraw/tldraw)
- [tailwindcss](https://github.com/tailwindlabs/tailwindcss)
- [vercel](https://github.com/vercel/vercel)

These lists are by no means exhaustive. Feel free to add to them with other strategies.

## Publishing `turbo` to the npm registry

See [the publishing guide](./release.md#release-turborepo).

## Adding A New Crate

When adding a new crate to the repo, it is essential that it is included/excluded from the
relevant workflows. This ensures that changes to the crate are tested by the correct workflows,
but that they do not trigger unnecessary workflows as well.

First, determine whether the crate is for Turbopack or Turborepo. If it is for Turbopack, then the crate
should be added to the `default-members` key in the root `Cargo.toml`. If the crate is for Turborepo, the
crate must be added to the `PATTERNS` list in "Turborepo related changes" section of the `test.yml`
workflow file. It must also be excluded from the "Turbopack related changes" section of the
`test.yml` workflow file.

For instance, if we were adding a `turborepo-foo` crate, we would add the following patterns:

```diff
      - name: Turbopack related changes
        id: turbopack
        uses: technote-space/get-diff-action@v6
        with:
          PATTERNS: |
            pnpm-lock.yaml
            package.json
            crates/**
            xtask/**
            .cargo/**
            rust-toolchain
            !crates/turborepo/**
            !crates/turborepo-lib/**
            !crates/turborepo-ffi/**
            !crates/turbo-updater/**
+           !crates/turborepo-foo/**
            !**.md
            !**.mdx

      - name: Turborepo related changes
        id: turborepo
        uses: technote-space/get-diff-action@v6
        with:
          PATTERNS: |
            pnpm-lock.yaml
            package.json
            crates/turborepo/**
            crates/turborepo-lib/**
            crates/turborepo-ffi/**
            crates/turbo-updater/**
+           crates/turborepo-foo/**
            .cargo/**
            rust-toolchain
            !**.md
            !**.mdx
```

The crate must also be explicitly excluded from build commands
for Turbopack and included in build commands for Turborepo.
To do so, add a `--exclude turborepo-foo` flag to the Turbopack commands in
`.cargo/config.toml` such as `tp-test`, and add an `-p turborepo-foo` to the Turborepo
commands such as `tr-test`.

Finally, the crate must be added to the Turborepo section of CODEOWNERS:

```diff
# overrides for crates that are owned by turbo-oss
  /crates/turborepo @vercel/turbo-oss
  /crates/turborepo-ffi @vercel/turbo-oss
+ /crates/turborepo-foo @vercel/turbo-oss
  /crates/turborepo-lib @vercel/turbo-oss
  /crates/turborepo-scm @vercel/turbo-oss
  /crates/turbo-updater @vercel/turbo-oss
```

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
