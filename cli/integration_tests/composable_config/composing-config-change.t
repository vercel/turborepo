Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# The override-values-task task in the root turbo.json has ALL the config. The workspace config
# defines the task and overrides all the keys. The tests below use `override-values-task` to assert that:
# - `outputs`, `inputs`, `env`, and `outputMode` are overriden from the root config.

# 1. First run, assert cache miss
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "29e0c6e610c3d010"

2. Run again and assert cache hit, and that full output is displayed
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "29e0c6e610c3d010"
3. Change turbo.json and assert cache miss
  $ cp $TARGET_DIR/apps/config-change/turbo-changed.json $TARGET_DIR/apps/config-change/turbo.json
  $ ${TURBO} run config-change-task --filter=config-change --dry=json | jq .tasks[0].hash
  "something-else"
