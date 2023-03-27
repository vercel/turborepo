Setup
  $ . ${TESTDIR}/../../setup.sh
  $ . ${TESTDIR}/../setup.sh $(pwd)

  $ rm -rf .turbo/runs

# Turbo exits early and doesn't generate run summaries on errors, so we need to use --continue for this test.
The maybefails task fails for one workspace but not the other
  $ TURBO_RUN_SUMMARY=true ${TURBO} run maybefails --continue > /dev/null
  my-app:maybefails: command finished with error, but continuing...

# ExitCode here is 1, because npm will report all errors with exitCode 1
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#maybefails")) | .[0].execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "buildFailed",
    "error": {},
    "exitCode": 1
  }

# This one has success exit code
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "util#maybefails")) | .[0].execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null,
    "exitCode": 0
  }
