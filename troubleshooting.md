# Troubleshooting

These are some common issues when starting.

## “cargo-nextest” cannot be opened because the developer cannot be verified.

On Macs, unsigned binaries cannot be run by default. To manually approve this
app, go to: Apple menu > System Preferences, click Security & Privacy , then
click General. You'll see "“cargo-nextest” was blocked from use because it is
not from an identified developer". Click the Allow Anyway button, and
`cargo-nexttest` can be run on the next invocation.

See https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac

## Cannot `yarn install` because of oniguruma

Oh boy, the classic python vs python3 blunder! And MacOS isn't providing either
by default anymore!  We need to setup your python environment, much like we
would setup your node env:

```shell
brew install pyenv
pyenv install 2.7.18
pyenv local 2.7.18
eval "$(pyenv init --path)"
```

Now try `yarn install` again.

See https://stackoverflow.com/a/67274521.
