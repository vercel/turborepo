Thank you for your interest in contributing to Turborepo!

- [General dependencies](#general-dependencies)
- [Optional dependencies](#optional-dependencies)
- [Structure of the repository](#structure-of-the-repository)
- [Building Turborepo](#building-turborepo)
  - [TLS Implementation](#tls-implementation)
- [Running tests](#running-tests)
- [Manually testing `turbo`](#manually-testing-turbo)
  - [Repositories to test with](#repositories-to-test-with)
- [Debugging tips](#debugging-tips)
  - [Links in error messages](#links-in-error-messages)
  - [Verbose logging](#verbose-logging)
  - [Crash logs](#crash-logs)
  - [Terminal UI debugging](#terminal-ui-debugging)
- [Publishing `turbo` to the npm registry](#publishing-turbo-to-the-npm-registry)
- [Contributing to examples](#contributing-to-examples)
  - [Contributing to an existing example](#contributing-to-an-existing-example)
  - [Philosophy for new examples](#philosophy-for-new-examples)
    - [Designing a new example](#designing-a-new-example)
  - [Testing examples](#testing-examples)

## General dependencies

You will need to have these dependencies installed on your machine to work on this repository:

- [Rust](https://www.rust-lang.org/tools/install) ([Repository toolchain](https://github.com/vercel/turborepo/blob/main/rust-toolchain.toml))
- [NodeJS](https://nodejs.org/en) v20
- [pnpm](https://pnpm.io/) v8
- [protoc](https://grpc.io/docs/protoc-installation/)
- [capnp](https://capnproto.org)

### Optional dependencies

- For running tests locally, `jq` and `zstd` are also required.
  - macOS: `brew install jq zstd`
  - Linux: `sudo apt update && sudo apt install jq zstd`
  - Windows: `choco install jq zstandard`
- On Linux, ensure LLD (LLVM Linker) is installed, as it's not installed by default on many Linux distributions (e.g. `apt install lld`).

## Structure of the repository

In general, there are two major areas in the repository:

- The `crates` directory with the Rust code for the `turbo` binary
- The `packages` directory with JavaScript packages
- The `examples` directory with examples of how to use Turborepo with other tools and frameworks
- The `docs` directory with the documentation for Turborepo

## Building Turborepo

1. Run `pnpm install` at the root of the repository
2. Run `cargo build`

### TLS Implementation

Turborepo uses [`reqwest`](https://docs.rs/reqwest/latest/reqwest/) to make requests to the Remote Cache.

`reqwest` supports two TLS
implementations: `rustls` and `native-tls`. `rustls` is a pure Rust implementation of TLS, while `native-tls`
is a wrapper around OpenSSL. You may select which implementation you want with the `native-tls`
and `rustls-tls` features.

By default, the `rustls-tls` feature is selected so that `cargo build` works
out of the box. If you wish to select `native-tls`, you may do so by running `cargo build --no-default-features --features native-tls`.

## Running tests

> [!IMPORTANT]
> You will need to have `jq` and `zstd` installed on your system in order to run tests. See [General dependencies](#general-dependencies) for instructions on how to install these tools.

First, install Turborepo globally with your package manager of choice. For instance, with npm, `npm install -g turbo`. This will install the `turbo` binary in your system's `PATH`, making it globally available.

Now, from the root directory, you can run:

- Unit tests

```bash
  cargo test
```

- A module's unit tests

```bash
cargo test -p <module>
```

- Integration tests
  ```bash
  pnpm test -- --filter=turborepo-tests-integration
  ```
- A single integration test
  e.g., to run everything in `turborepo-tests/integration/tests/run-summary`:

  ```bash
  # Build `turbo` first because the next command doesn't run through `turbo`
  pnpm -- turbo run build --filter=cli
  pnpm test -F turborepo-tests-integration -- "run-summary"
  ```

- Updating integration tests

  ```bash
  turbo run build --filter=cli
  pnpm --filter turborepo-tests-integration test:interactive
  ```

  You can pass a test name to run a single test, or a directory to run all tests in that directory.

  ```bash
  pnpm --filter turborepo-tests-integration test:interactive tests/turbo-help.t
  ```

## Manually testing `turbo`

After [building `turbo`](#building-turborepo), you can manually test `turbo` for the behaviors you're affecting with your changes. We recommend setting an alias to the built binary so you can call it with your alias.

```bash
alias devturbo='~/projects/turbo/target/debug/turbo'
devturbo run build --skip-infer
```

> [!IMPORTANT]
> The `--skip-infer` flag is required so that `turbo` doesn't try to use a locally installed binary of `turbo`. Forgetting to use this flag will cause `devturbo` to defer to the binary installed into the repository.

A non-exhaustive list of things to check on:

- Features related to your changes
- Test with and without daemon
- Installation scenarios
  - Global only. `turbo` is installed as global binary without a local `turbo` in repository.
  - Local only. `turbo` is installed as local binary without global `turbo` in PATH. `turbo` is invoked via a root package
    script.
  - Global and local. `turbo` is installed as global binary, and local `turbo` in repository. Global `turbo` delegates to
    local `turbo`

### Repositories to test with

There are many open-source Turborepos out in the community that you can test with. A few are listed below:

- [Next.js](https://github.com/vercel/next.js)
- [tldraw](https://github.com/tldraw/tldraw)
- [Tailwind CSS](https://github.com/tailwindlabs/tailwindcss)
- [Vercel CLI](https://github.com/vercel/vercel)
- This repository! Keep in mind that you'll be building and running `turbo` in the same repository, which can be confusing at times.

## Debugging tips

### Links in error messages

Many of Turborepo's error messages include links to information or documentation to help end users.

The base URL for the links can be set to a value of your choosing by providing a `TURBO_SITE` environment variable at compilation time.

```bash
TURBO_SITE="http://localhost:3000" cargo build
```

### Verbose logging

Verbose logging can be enabled by using the `-v`, `-vv`, or `-vvv` flag on your `turbo` command, depending on the level of logging you're looking for:

```bash
turbo build --vvv
```

### Crash logs

In the event of a crash, Rust's crash logs will be written to your temp directory. When `turbo` crashes, the location of the crash log will be printed to the console.

### Terminal UI debugging

The architecture of the Terminal UI makes for a tricky debugging experience. Because the UI writes to the console through `stdout` in a specific way, using `println!()` statements won't work as expected.

Instead, use `eprintln!()` to print to `stdout` and output `stdout` to a file:

```bash
# devturbo is an alias to the debug binary of `turbo` in this case
devturbo run build --ui=tui --skip-infer 2&> ~/tmp/logs.txt
```

> [!IMPORTANT]
> The `--skip-infer` flag is required so that `turbo` doesn't try to use a locally installed binary of `turbo`. Forgetting to use this flag will cause `devturbo` to defer to the binary installed into the repository rather than the one you're developing.

## Publishing `turbo` to the npm registry

See [the publishing guide](./RELEASE.md).

## Contributing to examples

Contributing to examples helps the Turborepo community by showcasing how to use Turborepo in real-world scenarios with other tools and frameworks. They can be found in [the examples directory](https://github.com/vercel/turborepo/tree/main/examples) of this repository.

> [!IMPORTANT]
> As Turborepo usage has grown, the community has contributed more and more examples to the repository. While this is exciting for us on the core team, we're unable to maintain the full surface area of every example, given the constant updates across the breadth of tooling that Turborepo works with in the examples.
>
> Because of this, a handful of the examples are explicitly marked as maintained by the core team. For the rest, we work with the community to keep them as up to date and correct as possible. If you find a problem with a community-supported template, we ask that you do not open a GitHub Issue for it. Instead, please open a pull request with the needed fixes.

The `basic` example is the default used by `create-turbo`.

For simplicity, each example is treated as a standalone "repository", separate from the rest of the repository, with its own dependencies, lockfile, `turbo` version, etc. You are able to run code and make code updates in an example without needing to install the dependencies of the rest of the repository.

> [!NOTE]
> You may find that opening your code editor directly in the example's directory that you're working on can give you a better sense of how the example will feel to community members who download the example.

### Contributing to an existing example

To contribute to an existing example, create your code updates and submit a pull request to the repository. No special steps are required to contribute to an example.

### Philosophy for new examples

Turborepo works with any framework, tool, or even language. Because of this, the community often expresses interest in creating new examples to showcase Turborepo working with other tooling.

However, we aim to have as few examples in the repository while still showcasing Turborepo's flexibility. By having fewer examples, the core team has a better chance to maintain the collection of examples, keeping them at a higher quality. The ecosystem evolves quickly, and keeping every example up-to-date for every tool requires a wealth of attention. Our goal is to balance the needs of the core team and the community together to keep the Turboverse in a healthy state.

Due to these considerations, we ask that you first [open a Discussion](https://github.com/vercel/turborepo/discussions/categories/ideas) before working on a new example for the repository. It's best to make sure ahead of time that the example you'd like to propose will be accepted. Once you have received approval, you can work on and create a pull request for your example.

#### Designing a new example

Each example should have a specific focus when compared to the `basic` example. The goal is for an example to show how to use a singular, distinct technology's usage in a Turborepo.

You're encouraged to start with the [`basic` example](https://github.com/vercel/turborepo/tree/main/examples/basic) and add your specific tool of interest to it. Each example should have as few modifications to the `basic` example as possible required to showcase the tool or framework.

Key characteristics of a great example include:

- One technology added to the `basic` example
- An updated README at the root of the example directory. Make sure to include any steps required to run the example
- All tasks in `turbo.json` in the example run successfully without any code changes needed
- Works with every package manager listed in our [Support Policy](https://turborepo.com/docs/getting-started/support-policy#package-managers)

Once you've created your example (with prior approval, as discussed above), you can submit a pull request to the repository.

### Testing examples

To test out the experience of your example with `create-turbo`, run `create-turbo` with the `--example` flag pointed to a URL to your example's source code:

```bash
npx create-turbo@latest --example https://github.com/your-org/your-repo/tree/your-branch/...
```

This will allow you to use the example as a user would.
