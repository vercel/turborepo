# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/10-too-many

  $ ${TURBO} run build --concurrency=1
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 1. Set --concurrency to at least 3
  [1]

  $ ${TURBO} run build --concurrency=2
  No token found for https://vercel.com/api. Run `turbo link` or `turbo login` first.
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 2. Set --concurrency to at least 3
  [1]
