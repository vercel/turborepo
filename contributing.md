## Testing

Install `cargo-nextest` (https://nexte.st/)

Run via:
```shell
cargo nextest run
```

For the test cases you need to run `yarn` in `crates/turbopack/tests/node-file-trace` to install some node_modules.

You can also create a little demo app and run
```shell
cargo run -p node-file-trace -- print demo/index.js
```
