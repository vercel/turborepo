# Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) 10-too-many

  $ ${TURBO} run build --concurrency=1
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 1. Set --concurrency to at least 3
  Turbo error: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 1. Set --concurrency to at least 3
  [1]

  $ ${TURBO} run build --concurrency=2
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 2. Set --concurrency to at least 3
  Turbo error: error preparing engine: Invalid persistent task configuration:
  You have 2 persistent tasks but `turbo` is configured for concurrency of 2. Set --concurrency to at least 3
  [1]
