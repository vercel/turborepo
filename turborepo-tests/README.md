# Turborepo Tests

## Integration tests

Integration tests verify turbo's behavior in realistic monorepo scenarios. There are two test systems:

### Prysk Tests (Legacy)

The original integration tests in `integration/tests/` use [prysk](https://www.prysk.net/), a Python-based test runner. These tests are `.t` files that execute shell commands and assert on output.

To run prysk tests:

```sh
pnpm test --filter=prysk -- integration/tests/run/force.t
```

### Testcontainers Tests (New)

A new test system using Docker containers is being developed in `crates/turborepo-integration-tests/`. These tests:

- Run as part of `cargo test`
- Use testcontainers for isolated Docker environments
- Use insta for snapshot testing

To run testcontainers tests:

```sh
cargo test -p turborepo-integration-tests --features integration-tests
```

See `crates/turborepo-integration-tests/README.md` for details.

## Example tests

These tests are for ensuring that the examples we are providing to users work right out of the box.

### Test structure

We guarantee that the examples work by running the `lint` and `build` tasks in each example and checking for a `>>> FULL TURBO` on the second run. This requires that the tasks pass and are cacheable. The logic for the tests is in `setup_example_test.sh` and takes a few steps:

1. `cd` to the example directory.
2. Install packages.
3. Run `turbo build lint`.
4. Run `turbo build lint` again and write the log results to a temporary text file.
5. Read that text file looking for a `>>> FULL TURBO`.

### Creating a test

To create a test for a new example:

1. Copy the `basic` directory into a new folder.
2. Edit the `name` field in `package.json`.
3. Run `pnpm install` for the repository so the new package is added to the workspace.
4. Edit the `test` script to path to the directory you are interested in and use the package manager for that example.
5. Run `turbo test --filter="@turborepo-examples-tests/*"` in your terminal to make sure all is well!
