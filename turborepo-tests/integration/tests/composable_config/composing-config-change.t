Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) composable_config

# 1. First run, check the hash
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "ac8ef319babcdfdc"

2. Run again and assert task hash stays the same
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "ac8ef319babcdfdc"

3. Change turbo.json and assert that hash changes
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "b94c5e8c556d70cb"
