Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh

Copy fixture files
  $ mkdir -p ${TARGET_DIR}/packages/dependent-task-hashing
  $ cp ${TESTDIR}/turbo.json ${TARGET_DIR}/packages/dependent-task-hashing/turbo.json
  $ cp ${TESTDIR}/package.json ${TARGET_DIR}/packages/dependent-task-hashing/package.json

First run - everything should be a cache miss
  $ ${TURBO} run build --filter=dependent-task-hashing | grep -Eo "(FULL TURBO|cache miss|cache hit)"
  cache miss
  cache miss

Second run - should be a cache hit for both tasks
  $ ${TURBO} run build --filter=dependent-task-hashing | grep -Eo "(FULL TURBO|cache miss|cache hit)"
  cache hit
  cache hit
  FULL TURBO
