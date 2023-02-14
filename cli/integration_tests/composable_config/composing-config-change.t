Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# 1. First run, check the hash
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "b17ced7629048d97"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "b17ced7629048d97"

3. Change turbo.json and assert that hash changes
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "6c56b35e06abb856"
