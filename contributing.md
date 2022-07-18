`turbo-tooling` uses [Cargo workspaces][workspaces] monorepo. You'll find
several workspaces inside the `creates/` directory. In order to run a particular
create, you can use the `cargo run -p [CRATE_NAME]` command.

## Testing

Install `cargo-nextest` (https://nexte.st/)

Run via:
```shell
cargo nextest run
```

For the test cases you need to run `yarn` in `crates/turbopack/tests/node-file-trace` to install some node_modules.

If running `yarn` fails on macOS, you might need to install the following packages: `python`, `pkg-config`, `pixman`, `cairo`, `pango`. If you're running Zsh and Homebrew, you can run the following commands before running `yarn`.
```
brew install python@3.9 pkg-config pixman cairo pango
echo 'export PATH="/opt/homebrew/opt/python@3.9/libexec/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

You can also create a little demo app and run
```shell
cargo run -p node-file-trace -- print demo/index.js
```

[workspaces]: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
