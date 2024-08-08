# Contributing to Turbo

Thanks for your interest in contributing to Turbo!

- [Contributing to Turbo](#contributing-to-turbo)
  - [General Dependencies](#general-dependencies)
    - [Linux Dependencies](#linux-dependencies)
  - [Contributing to Turborepo](#contributing-to-turborepo)
    - [Building Turborepo](#building-turborepo)
    - [TLS Implementation](#tls-implementation)
    - [Running Turborepo Tests](#running-turborepo-tests)
      - [Turborepo Tests](#turborepo-tests)
  - [Debugging Turborepo](#debugging-turborepo)
  - [Benchmarking Turborepo](#benchmarking-turborepo)
  - [Updating `turbo`](#updating-turbo)
  - [Manually testing `turbo`](#manually-testing-turbo)
  - [Publishing `turbo` to the npm registry](#publishing-turbo-to-the-npm-registry)
  - [Creating a new release blog post](#creating-a-new-release-blog-post)
  - [Troubleshooting](#troubleshooting)

## General Dependencies

- [Rust](https://www.rust-lang.org/tools/install)
- [cargo-groups](https://github.com/nicholaslyang/cargo-groups)

### Linux Dependencies

- LLD (LLVM Linker), as it's not installed by default on many Linux distributions (e.g. `apt install lld`).

## Contributing to Turborepo

### Building Turborepo

Dependencies

1. Install [turborepo crate](./crates/turborepo/README.md) build requirements

1. Run `pnpm install` at root

Building

- Building `turbo` CLI: `cargo build -p turbo`
- Using `turbo` to build `turbo` CLI: `./turbow.js`

### TLS Implementation

Turborepo uses `reqwest`, a Rust HTTP client, to make requests to the Turbo API. `reqwest` supports two TLS
implementations: `rustls` and `native-tls`. `rustls` is a pure Rust implementation of TLS, while `native-tls`
is a wrapper around OpenSSL. Turborepo allows users to select which implementation they want with the `native-tls`
and `rustls-tls` features. By default, the `rustls-tls` feature is selected---this is done so that `cargo build` works
out of the box. If you wish to select `native-tls`, you may do so by passing `--no-default-features --features native-tls`
to the build command.

### Running Turborepo Tests

Install dependencies

On macOS:

```bash
brew install jq zstd
```

#### Turborepo Tests

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
- A single Integration test
  e.g to run everything in `tests/run-summary`:

  ```
  # build first because the next command doesn't run through turbo
  pnpm -- turbo run build --filter=cli
  pnpm test -F turborepo-tests-integration -- "run-summary"
  ```

  Note: this is not through turbo, so you'll have to build turbo yourself first.

- Example tests
  ```bash
  pnpm test -- --filter=turborepo-tests-examples -- <example-name> <packagemanager>
  ```

## Debugging Turborepo

1. Install `go install github.com/go-delve/delve/cmd/dlv@latest`
1. In VS Code's "Run and Debug" tab, select `Build Basic` to start debugging the initial launch of `turbo` against the `build` target of the Basic Example. This task is configured in [launch.json](./.vscode/launch.json).

## Benchmarking Turborepo

Follow the instructions in the [`benchmark/README.md`](./benchmark/README.md).

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
  - `pnpm dlx create-turbo@latest prune-test --package-manager pnpm && cd prune-test`
  - `turbo --skip-infer prune docs && cd out && pnpm install --frozen-lockfile`
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

## Creating a new release blog post

Creating a new release post can be done via a turborepo generator. Run the following command from anywhere within the repo:

```bash
turbo generate run "blog - release post"
```

This will walk you through creating a new blog post from start to finish.

NOTE: If you would like to update the stats (github stars / npm downloads / time saved) for an existing blog post that has yet to be published (useful if time has passed since the blog post was created, and up to date stats are required before publishing) - run:

```bash
turbo generate run "blog - "blog - update release post stats"
```

and choose the blog post you would like to update.

## Troubleshooting

See [Troubleshooting][].

[workspaces]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[troubleshooting]: troubleshooting.md
