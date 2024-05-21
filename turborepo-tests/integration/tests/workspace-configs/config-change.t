Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# 1. First run, check the hash
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "e0471b5eddce1aab"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "e0471b5eddce1aab"

3. Change turbo.json and assert that hash changes
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "41e50d2dc738d0f8"
