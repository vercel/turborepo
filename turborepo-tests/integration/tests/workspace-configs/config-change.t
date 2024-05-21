Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# 1. First run, check the hash
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "38f031e7ff4d4089"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "38f031e7ff4d4089"

3. Change turbo.json and assert that hash changes
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "e100adf0b01751c4"
