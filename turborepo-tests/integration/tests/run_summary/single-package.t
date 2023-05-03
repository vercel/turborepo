Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) single_package

Check
  $ ${TURBO} run build --summarize > /dev/null
  $ test -d .turbo/runs
  $ ls .turbo/runs/*.json | wc -l
  \s*1 (re)

  $ source "$TESTDIR/../_helpers/run-summary-utils.sh"
  $ SUMMARY=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ TASK_SUMMARY=$(getSummaryTask "$SUMMARY" "build")

  $ cat $SUMMARY | jq '.tasks | length'
  1
  $ cat $SUMMARY | jq '.version'
  "0"
  $ cat $SUMMARY | jq '.execution | keys'
  [
    "attempted",
    "cached",
    "command",
    "endTime",
    "exitCode",
    "failed",
    "repoPath",
    "startTime",
    "success"
  ]

  $ cat $SUMMARY | jq 'keys'
  [
    "envMode",
    "execution",
    "globalCacheInputs",
    "id",
    "scm",
    "tasks",
    "turboVersion",
    "version"
  ]

  $ cat $SUMMARY | jq '.scm'
  {
    "type": "git",
    "sha": "[a-z0-9]+", (re)
    "branch": ".+" (re)
  }

  $ cat $SUMMARY | jq '.execution.exitCode'
  0
  $ cat $SUMMARY | jq '.execution.attempted'
  1
  $ cat $SUMMARY | jq '.execution.cached'
  0
  $ cat $SUMMARY | jq '.execution.failed'
  0
  $ cat $SUMMARY | jq '.execution.success'
  1
  $ cat $SUMMARY | jq '.execution.startTime'
  [0-9]+ (re)
  $ cat $SUMMARY | jq '.execution.endTime'
  [0-9]+ (re)

  $ echo $TASK_SUMMARY | jq 'keys'
  [
    "cache",
    "cliArguments",
    "command",
    "dependencies",
    "dependents",
    "envMode",
    "environmentVariables",
    "excludedOutputs",
    "execution",
    "expandedOutputs",
    "framework",
    "hash",
    "hashOfExternalDependencies",
    "inputs",
    "logFile",
    "outputs",
    "resolvedTaskDefinition",
    "task",
    "taskId"
  ]

  $ echo $TASK_SUMMARY | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "exitCode": 0
  }
  $ echo $TASK_SUMMARY | jq '.cliArguments'
  []
  $ echo $TASK_SUMMARY | jq '.expandedOutputs'
  [
    ".turbo/turbo-build.log",
    "foo"
  ]
  $ echo $TASK_SUMMARY | jq '.cache'
  {
    "local": false,
    "remote": false,
    "status": "MISS",
    "timeSaved": 0
  }
