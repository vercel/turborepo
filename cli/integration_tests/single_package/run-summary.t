Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --summarize > /dev/null
  $ test -d .turbo/runs
  $ ls .turbo/runs/*.json | wc -l
  \s*1 (re)

  $ source "$TESTDIR/../run-summary-utils.sh"
  $ SUMMARY=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ TASK_SUMMARY=$(getSummaryTask "$SUMMARY" "build")

  $ cat $SUMMARY | jq '.tasks | length'
  1
  $ cat $SUMMARY | jq '.version'
  "0"
  $ cat $SUMMARY | jq '.executionSummary | keys'
  [
    "attempted",
    "cached",
    "endTime",
    "exitCode",
    "failed",
    "startTime",
    "success"
  ]

  $ cat $SUMMARY | jq '.executionSummary.exitCode'
  0
  $ cat $SUMMARY | jq '.executionSummary.attempted'
  1
  $ cat $SUMMARY | jq '.executionSummary.cached'
  0
  $ cat $SUMMARY | jq '.executionSummary.failed'
  0
  $ cat $SUMMARY | jq '.executionSummary.success'
  1
  $ cat $SUMMARY | jq '.executionSummary.startTime'
  [0-9]+ (re)
  $ cat $SUMMARY | jq '.executionSummary.endTime'
  [0-9]+ (re)

  $ echo $TASK_SUMMARY | jq 'keys'
  [
    "cacheState",
    "command",
    "commandArguments",
    "dependencies",
    "dependents",
    "environmentVariables",
    "excludedOutputs",
    "execution",
    "expandedInputs",
    "expandedOutputs",
    "framework",
    "hash",
    "hashOfExternalDependencies",
    "logFile",
    "outputs",
    "resolvedTaskDefinition",
    "task"
  ]

  $ echo $TASK_SUMMARY | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null,
    "exitCode": 0
  }
  $ echo $TASK_SUMMARY | jq '.commandArguments'
  []
  $ echo $TASK_SUMMARY | jq '.expandedOutputs'
  [
    ".turbo/turbo-build.log",
    "foo"
  ]
  $ echo $TASK_SUMMARY | jq '.cacheState'
  {
    "local": false,
    "remote": false
  }
