## Integration Tests

The `cli/integration_tests` directory contains tests for Turborepo, exercising builds of
Turborepo against custom monorepos and turbo.json setups. Tests are written using [`prysk`][1],
which executes the CLI and can execute arbitrary other commands to assert the result. Some tests
assert the log output, some assert the created artifacts, some assert configuration, etc etc.

### Adding new tests

To add a new test to this directory, create a `my-new-test.t` file. You will likely want to
the file to start with:

```bash
Setup
  $ . ${TESTDIR}/setup.sh
  $ . ${TESTDIR}/setup_monorepo.sh $(pwd)
```

- `setup.sh` sets a `TURBO` environment variable that points to the locally built binary
- `setup_monorepo.sh` uses one of the test repos in the `_fixtures` directory to exercise
  the `TURBO` binary against.

### Fixtures

For the most part, use the `basic_monorepo`, or `single_package` fixtures to test against.
You can add custom fixture monorepos as an escape hatch or if you truly need a custom monorepo.

#### Custom turbo.json

If an existing fixture meets your needs, but you need a custom `turbo.json`, create
a directory for your test (instead of just `my-new-test.t`), add your "local" fixture `turbo.json`
there, and then use `cp` as part the setup before writing your test. For example:

```bash
Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/../setup_monorepo.sh $(pwd)

Custom config
  $ cp ${TESTDIR}/myturboconfig.json $(pwd)/turbo.json
  $ git commit -am "Update turbo.json"

Write your tests
...
```

(Note that in the example above the paths to `setup.sh` and `setup_monorepo.sh` changed)
