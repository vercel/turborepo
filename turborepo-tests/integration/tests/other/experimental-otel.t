# Smoke tests for experimental OTEL configuration.
# These tests verify that enabling/disabling OTEL via environment variables and CLI flags
# does not break normal turbo run behavior. Exporter correctness is primarily covered
# by Rust unit tests in crates/turborepo-lib/src/config/experimental_otel.rs and
# crates/turborepo-lib/src/observability/otel.rs.

Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Smoke test: OTEL enabled via environment variables does not break turbo run
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
  $ export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=http://localhost:4318
  $ ${TURBO} run build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing .* (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`
 
  [0]

Smoke test: OTEL enabled via CLI flags does not break turbo run
  $ unset TURBO_EXPERIMENTAL_OTEL_ENABLED
  $ unset TURBO_EXPERIMENTAL_OTEL_ENDPOINT
  $ ${TURBO} run build --filter=my-app --experimental-otel-enabled --experimental-otel-endpoint=http://localhost:4318
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs .* (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s\s*>>> FULL TURBO (re)
  
  [0]

Smoke test: http/protobuf protocol flag is accepted without error
  $ ${TURBO} run build --filter=my-app --experimental-otel-enabled --experimental-otel-endpoint=http://localhost:4318 --experimental-otel-protocol=http-protobuf
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs .* (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s\s*>>> FULL TURBO (re)
  
  [0]

Smoke test: OTEL disabled via environment variable does not break turbo run
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=0
  $ export TURBO_EXPERIMENTAL_OTEL_ENDPOINT=http://localhost:4318
  $ ${TURBO} run build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs .* (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s\s*>>> FULL TURBO (re)
  
  [0]

Smoke test: enabled via env without endpoint is a no-op (exporter not configured)
  $ export TURBO_EXPERIMENTAL_OTEL_ENABLED=1
  $ unset TURBO_EXPERIMENTAL_OTEL_ENDPOINT
  $ ${TURBO} run build --filter=my-app
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache hit, replaying logs .* (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s\s*>>> FULL TURBO (re)
  
  [0]

