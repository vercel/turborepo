# Troubleshooting

These are some common issues when starting.

## “cargo-nextest” cannot be opened because the developer cannot be verified.

On Macs, unsigned binaries cannot be run by default. To manually approve this
app, go to: Apple menu > System Preferences, click Security & Privacy , then
click General. You'll see "“cargo-nextest” was blocked from use because it is
not from an identified developer". Click the Allow Anyway button, and
`cargo-nexttest` can be run on the next invocation.

See https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac

## Cannot `pnpm install` because of oniguruma

Oniguruma doesn't yet provide a prebuild binary for arm64 MacOS. And, MacOS
isn't providing a python2 environment by default anymore! We need to setup your
python environment, much like we would setup your node env:

```shell
brew install pyenv
eval "$(pyenv init --path)"
pyenv install 2.7.18
pyenv local 2.7.18
```

Now try `pnpm install` again.

See the `canvas` tip below, and https://stackoverflow.com/a/67274521.

## Cannot `pnpm install` because of canvas

Canvas also doesn't provide an arm64 prebuilt binary. To manually compile,
you'll need to install the following packages:

If running `pnpm` fails on macOS, you might need to install the following packages: `python`, `pkg-config`, `pixman`, `cairo`, `pango`. If you're running Zsh and Homebrew, you can run the following commands before running `pnpm`.

```shell
brew install pkg-config pixman cairo pango
```

Now try `pnpm install` again.

See the `oniguruma` tip above, and https://github.com/Automattic/node-canvas/blob/master/Readme.md#compiling.
