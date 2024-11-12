# Troubleshooting

These are some common issues when starting.

## “cargo-nextest” cannot be opened because the developer cannot be verified.

On Macs, unsigned binaries cannot be run by default. To manually approve this
app, go to: [Apple menu > System Preferences, click Security & Privacy, under the General tab](x-apple.systempreferences:com.apple.preference.security). You'll see "“cargo-nextest” was blocked from use because it is
not from an identified developer". Click the "Allow Anyway" button, and
`cargo-nextest` can be run on the next invocation.

See also: https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac

## Cannot `pnpm install` because of oniguruma

Oniguruma does not provide a prebuilt binary for arm64 macOS. Further, macOS
does not provide a python2 environment by default anymore! You need to setup your
python environment, much like we would setup your node environment:

```shell
brew install pyenv
eval "$(pyenv init --path)"
pyenv install 2.7.18
pyenv local 2.7.18
```

Now try `pnpm install` again.

See also: https://stackoverflow.com/a/67274521

## Cannot `pnpm install` because of canvas

Canvas does not provide a prebuilt binary for arm64. To manually compile,
you can use Homebrew to install the necessary packages:

```shell
brew install python pkg-config pixman cairo pango
```

Now try `pnpm install` again.

See also: https://github.com/Automattic/node-canvas/blob/master/Readme.md#compiling

## Enabling logging in Turborepo

Logging can be enabled in two ways, first with the verbosity flag (-vvv) which
sets the global log level, but it is also possible to use the TURBO_LOG_VERBOSITY
environment variable. With this, you can set different log levels per module.
For syntax, see the [Env Filter Syntax][1]

[1][https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html]

## Failing integration tests due to hash changes

If a lot of integration tests are failing with changes in the hash for `package-lock.json`,
you might be using an old version of `npm`. We try to set it in the test (`setup_package_manager.sh` and
`setup_integration_test.sh`), but if your version is too old, it might not work.
In which case, upgrade it to whatever the GitHub Actions runner uses.
