# Turborepo Tests

## Integration tests

TODO

## Terminal UI tests

The `tui` package uses [`terminal-control`](https://github.com/anomalyco/terminal-control) to run Ubuntu-focused black-box sanity tests against a real `turbo` binary in a pseudo-terminal. The suite covers rendering, keyboard navigation, interactive tasks, resizing, log streaming, and terminal restoration.

Build the debug binary before running the tests locally:

```bash
cargo build --package turbo
TURBO_BINARY_PATH="$PWD/target/debug/turbo" turbo run test:tui --filter=@turbo/tui-tests
```

Known failing regressions are skipped by default. Set `TURBO_TEST_KNOWN_BUGS=1` to execute their expected-correct assertions while reproducing a bug locally.
