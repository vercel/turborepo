# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/10-too-many

  $ ${TURBO} run build --concurrency=1
    x Invalid task configuration
    `->   x You have 2 persistent tasks but `turbo` is configured for
          | concurrency of 1. Set --concurrency to at least 3
  
  [1]

  $ TURBO_CONCURRENCY=1 ${TURBO} run build
    x Invalid task configuration
    `->   x You have 2 persistent tasks but `turbo` is configured for
          | concurrency of 1. Set --concurrency to at least 3
  
  [1]

  $ ${TURBO} run build --concurrency=2
    x Invalid task configuration
    `->   x You have 2 persistent tasks but `turbo` is configured for
          | concurrency of 2. Set --concurrency to at least 3
  
  [1]

  $ TURBO_CONCURRENCY=2 ${TURBO} run build
    x Invalid task configuration
    `->   x You have 2 persistent tasks but `turbo` is configured for
          | concurrency of 2. Set --concurrency to at least 3
  
  [1]

  $ ${TURBO} run build --concurrency=3 > tmp.log 2>&1
  $ grep -E "2 successful, 2 total" tmp.log
   Tasks:    2 successful, 2 total

  $ TURBO_CONCURRENCY=3 ${TURBO} run build > tmp-env.log 2>&1
  $ grep -E "2 successful, 2 total" tmp-env.log
   Tasks:    2 successful, 2 total

