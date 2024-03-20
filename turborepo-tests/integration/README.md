## Integration Tests

The `turborepo-tests/integration` directory contains tests for Turborepo, exercising builds of
Turborepo against custom monorepos and turbo.json setups. Tests are written using [`prysk`][1],
which executes the CLI and can execute arbitrary other commands to assert the result. Some tests
assert the log output, some assert the created artifacts, some assert configuration, etc etc.

### Adding new tests

To add a new test to this directory, create a `my-new-test.t` file. You will likely want to
the file to start with:

```bash
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
```

`setup_integration_test.sh` sets a `TURBO` environment variable that points to the locally built binary.
In a Prysk context `$(pwd)`, is a tmp directory that prysk creates, and a fixture will be copied
into this directory.

### Fixtures

For the most part, use the `basic_monorepo`, or `single_package` fixtures to test against.
By default the script will use `basic_monorepo`, but you can specify the fixture with a second
argument:

```bash
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh single_package
```

where `single_package` is a directory located at `turborepo-tests/integration/fixtures/single_package`.

You can also pass a second argument to change the packageManager of a fixture:

```bash
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh basic_monorepo "yarn@1.22.17"
```

Note that if you want to customize the package manager, you'll have to specify the fixture name
also since the script just uses positional arguments.

You can add custom fixture monorepos as an escape hatch or if you truly need a custom monorepo.

#### Custom turbo.json

If an existing fixture meets your needs, but you need a custom `turbo.json`, add your customized
turbo.json config in `turborepo-tests/integration/fixtures/turbo-configs` and use the helper
script to replace before your test runs:

```bash
Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh single_package

Custom config
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd) "my-custom-config.json"

Write your tests
...
```
