Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

# Delete all run summaries
  $ rm -rf .turbo/runs

  $ ${TURBO} run build --summarize -- someargs > /dev/null # first run (should be cache miss)

# HACK: Generated run summaries are named with a ksuid, which is a time-sorted ID. This _generally_ works
# but we're seeing in this test that sometimes a summary file is not sorted (with /bin/ls) in the order we expect
# causing intermittent test failures. 
# Add a sleep statement so we can be sure that the second run is a later timestamp,
# so we can reliably get it with `|head -n1` and `|tail -n1` later in this test.
# When we start emitting the path to the run summary file that was generated, or a way to specify
# the output file, we can remove this and look for the file directly.
# If you find this sleep statement, try running this test 10 times in a row. If there are no
# failures, it *should* be safe to remove.
  $ sleep 1
  $ ${TURBO} run build --summarize -- someargs > /dev/null # run again (expecting full turbo here)

# no output, just check for 0 status code, which means the directory was created
  $ test -d .turbo/runs
# expect 2 run summaries are created
  $ ls .turbo/runs/*.json | wc -l
  \s*2 (re)

# get jq-parsed output of each run summary
  $ FIRST=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ SECOND=$(/bin/ls .turbo/runs/*.json | tail -n1)

  $ cat $FIRST | jq 'keys'
  [
    "envMode",
    "execution",
    "globalCacheInputs",
    "id",
    "packages",
    "scm",
    "tasks",
    "turboVersion",
    "version"
  ]

# some top level run summary validation
  $ cat $FIRST | jq '.scm'
  {
    "type": "git",
    "sha": "[a-z0-9]+", (re)
    "branch": ".+" (re)
  }

  $ cat $FIRST | jq '.tasks | length'
  2
  $ cat $FIRST | jq '.version'
  "0"
  $ cat $FIRST | jq '.execution.exitCode'
  0
  $ cat $FIRST | jq '.execution.attempted'
  2
  $ cat $FIRST | jq '.execution.cached'
  0
  $ cat $FIRST | jq '.execution.failed'
  0
  $ cat $FIRST | jq '.execution.success'
  2
  $ cat $FIRST | jq '.execution.startTime'
  [0-9]+ (re)
  $ cat $FIRST | jq '.execution.endTime'
  [0-9]+ (re)

# Extract some task-specific summaries from each
  $ source "$TESTDIR/../_helpers/run-summary-utils.sh"
  $ FIRST_APP_BUILD=$(getSummaryTaskId "$FIRST" "my-app#build")
  $ SECOND_APP_BUILD=$(getSummaryTaskId "$SECOND" "my-app#build")
  $ FIRST_UTIL_BUILD=$(getSummaryTaskId "$FIRST" "util#build")

  $ echo $FIRST_APP_BUILD | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "exitCode": 0
  }
  $ echo $FIRST_APP_BUILD | jq '.cliArguments'
  [
    "someargs"
  ]
  $ echo $FIRST_APP_BUILD | jq '.hashOfExternalDependencies'
  "ccab0b28617f1f56"
  $ echo $FIRST_APP_BUILD | jq '.expandedOutputs'
  [
    "apps/my-app/.turbo/turbo-build.log"
  ]
# validate that cache state updates in second run
  $ echo $FIRST_APP_BUILD | jq '.cache'
  {
    "local": false,
    "remote": false,
    "status": "MISS",
    "timeSaved": 0
  }
  $ echo $SECOND_APP_BUILD | jq '.cache'
  {
    "local": true,
    "remote": false,
    "status": "HIT",
    "source": "LOCAL",
    "timeSaved": [0-9]+ (re)
  }

# Some validation of util#build
  $ echo $FIRST_UTIL_BUILD | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "exitCode": 0
  }

# another#build is not in tasks, because it didn't execute (script was not implemented)
  $ getSummaryTaskId $FIRST "another#build"
  null
