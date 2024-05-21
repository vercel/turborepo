Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# 1. First run, check the hash
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
<<<<<<< HEAD
  "e0471b5eddce1aab"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "e0471b5eddce1aab"
=======
  "c4d3e439d9786da3"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "c4d3e439d9786da3"
>>>>>>> 37c3c596f1 (chore: update integration tests)

3. Change turbo.json and assert that hash changes
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
<<<<<<< HEAD
  "41e50d2dc738d0f8"
=======
  "1f4ae40e6a7c2773"
>>>>>>> 37c3c596f1 (chore: update integration tests)
