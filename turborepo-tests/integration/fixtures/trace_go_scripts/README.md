# trace-go-scripts

Services whose `test` script starts `go test` in different ways: in a
directory below the package (`cd app &&`, `go -C engine`), through `gotestsum`,
for the modules of a `go.work`, and from a shell script file.

- `services/billing`: `cd app && go test ./...`; the module is `app`.
- `services/search`: `go -C engine test -race ./...`; the module is `engine`.
- `services/queue`: `gotestsum --format testname -- -tags=integration ./...`;
  the module is the package directory, and `tools` below it is a module of its
  own, which `./...` does not enter.
- `services/batch`: `go test ./one/... ./two/...` for the two modules of its
  `go.work`.
- `services/mail`: `./scripts/test.sh`, which runs `go test ./...`.
- `services/ops`: `./scripts/test`, a shell script without an extension that
  runs `go test ./...`.

The root package's `test` script is `turbo run test`. `tools/lint` is a Go
module in the root package that no script tests.
