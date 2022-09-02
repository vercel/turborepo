`turbo-tooling` uses [Cargo workspaces][workspaces] monorepo. You'll find
several workspaces inside the `crates/` directory. In order to run a particular
crate, you can use the `cargo run -p [CRATE_NAME]` command.

## Testing

Install `cargo-nextest` (https://nexte.st/)

Run via:

```shell
cargo nextest run
```

For the test cases you need to run `yarn` to install some node_modules. See [troubleshooting][] for solutions to common problems.

You can also create a little demo app and run

```shell
cargo run -p node-file-trace -- print demo/index.js
```

[workspaces]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
[troubleshooting]: troubleshooting.md
