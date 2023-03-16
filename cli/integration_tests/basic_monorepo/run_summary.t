Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build > /dev/null
# no output, just check for 0 status code
  $ test -d .turbo/runs
  $ ls .turbo/runs/*.json | wc -l
  \s*1 (re)

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].execution'
  {
    "start": "[0-9-:\.TZ]+", (re)
    "duration": [0-9]+, (re)
    "status": "built",
    "error": null
  }

  $ cat $(/bin/ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "util#build")) | .[0].execution'
  {
    "start": "[0-9-:\.TZ]+", (re)
    "duration": [0-9]+, (re)
    "status": "built",
    "error": null
  }

# validate expandedOutputs since it won't be in dry runs and we want some testing around that
  $ cat $(ls .turbo/runs/*.json | head -n1) | jq '.tasks | map(select(.taskId == "my-app#build")) | .[0].expandedOutputs'
  [
    "apps/my-app/.turbo/turbo-build.log"
  ]

# Without env var, no summary file is generated
  $ rm -rf .turbo/runs
  $ ${TURBO} run build > /dev/null
# validate with exit code so the test works on macOS and linux
  $ test -d .turbo/runs
  [1]
