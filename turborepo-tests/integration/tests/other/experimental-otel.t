Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Test that OTEL exporter can be enabled via environment variables
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
  $ export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=http://localhost:4318
  $ ${TURBO} run build --filter=my-app
  .*build.*my-app (re)
  [0]

Test that OTEL exporter can be enabled via CLI flags
  $ unset TURBO_EXPERIMENTAL_OTEL_ENABLED
  $ unset TURBO_EXPERIMENTAL_OTEL_ENDPOINT
  $ ${TURBO} run build --filter=my-app --experimental-otel-enabled --experimental-otel-endpoint=http://localhost:4318
  .*build.*my-app (re)
  [0]

Test that OTEL exporter works with http/protobuf protocol
  $ ${TURBO} run build --filter=my-app --experimental-otel-enabled --experimental-otel-endpoint=http://localhost:4318 --experimental-otel-protocol=http-protobuf
  .*build.*my-app (re)
  [0]

Test that OTEL exporter can be disabled via environment variable
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=0
  $ export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=http://localhost:4318
  $ ${TURBO} run build --filter=my-app
  .*build.*my-app (re)
  [0]

Test that OTEL exporter requires endpoint when enabled
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
  $ unset TURBO_EXPERIMENTAL_OTEL_ENDPOINT
  $ ${TURBO} run build --filter=my-app
  .*build.*my-app (re)
  [0]

