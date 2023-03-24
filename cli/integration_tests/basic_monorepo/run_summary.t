Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build -- someargs > /dev/null
# no output, just check for 0 status code
  $ test -d .turbo/runs
  $ ls .turbo/runs/*.json | wc -l
  \s*1 (re)

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | length'
  2

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.version'
  "0"

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.attempted'
  2
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.cached'
  0
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.failed'
  0
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.success'
  2
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.startTime'
  [0-9]+ (re)
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.executionSummary.endTime'
  [0-9]+ (re)

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null
  }
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].commandArguments'
  [
    "someargs"
  ]

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "util#build")) | .[0].execution'
  {
    "startTime": [0-9]+, (re)
    "endTime": [0-9]+, (re)
    "status": "built",
    "error": null
  }
  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].hashOfExternalDependencies'
  "ccab0b28617f1f56"

# validate expandedOutputs since it won't be in dry runs and we want some testing around that
  $ cat $(ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].expandedOutputs'
  [
    "apps/my-app/.turbo/turbo-build.log"
  ]

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "another#build"))'
  []

# Without env var, no summary file is generated
  $ rm -rf .turbo/runs
  $ ${TURBO} run build > /dev/null
# validate with exit code so the test works on macOS and linux
  $ test -d .turbo/runs
  [1]
