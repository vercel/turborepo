Setup
  $ . ${TESTDIR}/../../setup.sh
  $ . ${TESTDIR}/../setup.sh $(pwd)

# Delete all run summaries
  $ rm -rf .turbo/runs

  $ TURBO_RUN_SUMMARY=true ${TURBO} run build -- someargs > /dev/null # first run (should be cache miss)

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
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build -- someargs > /dev/null # run again (expecting full turbo here)

# no output, just check for 0 status code, which means the directory was created
  $ test -d .turbo/runs
# expect 2 run summaries are created
  $ ls .turbo/runs/*.json | wc -l
  \s*2 (re)

# get jq-parsed output of each run summary
  $ FIRST=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ SECOND=$(/bin/ls .turbo/runs/*.json | tail -n1)

# some top level run summary validation
  $ cat $FIRST | jq '.tasks | length'
  2
  $ cat $FIRST | jq '.version'
  "0"
  $ cat $FIRST | jq '.executionSummary.exitCode'
  0
  $ cat $FIRST | jq '.executionSummary.attempted'
  2
  $ cat $FIRST | jq '.executionSummary.cached'
  0
  $ cat $FIRST | jq '.executionSummary.failed'
  0
  $ cat $FIRST | jq '.executionSummary.success'
  2
  $ cat $FIRST | jq '.executionSummary.startTime'
  [0-9]+ (re)
  $ cat $FIRST | jq '.executionSummary.endTime'
  [0-9]+ (re)

# Extract some task-specific summaries from each
  $ source "$TESTDIR/../../run-summary-utils.sh"
  $ FIRST_APP_BUILD=$(getSummaryTaskId "$FIRST" "my-app#build")
  $ SECOND_APP_BUILD=$(getSummaryTaskId "$SECOND" "my-app#build")
  $ FIRST_UTIL_BUILD=$(getSummaryTaskId "$FIRST" "util#build")

  $ echo $FIRST_APP_BUILD | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null,
    "exitCode": 0
  }
  $ echo $FIRST_APP_BUILD | jq '.commandArguments'
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
  $ echo $FIRST_APP_BUILD | jq '.cacheState'
  {
    "local": false,
    "remote": false
  }
  $ echo $SECOND_APP_BUILD | jq '.cacheState'
  {
    "local": true,
    "remote": false
  }

# Some validation of util#build
  $ echo $FIRST_UTIL_BUILD | jq '.execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null,
    "exitCode": 0
  }

# another#build is not in tasks, because it didn't execute (script was not implemented)
  $ getSummaryTaskId $FIRST "another#build"
  null
