Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh
  $ . ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "invalid-interactive.json"

Only one persistent interactive task is allowed
  $ ${TURBO} run build
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  Tried to set persistent task .*#build as interactive, but .*#build is already set\. Only one persistent interactive task is allowed\. (re)
  Tried to set persistent task .*#build as interactive, but .*#build is already set\. Only one persistent interactive task is allowed\. (re)
  [1]

TODO: Validation applies even via workspace configs
