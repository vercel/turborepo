  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh "with_pkg_deps"
  $ source "$TESTDIR/../_helpers/run-summary-utils.sh"

  $ rm -rf .turbo/runs
  $ git commit --quiet -am "new sha" --allow-empty && ${TURBO} run build --summarize > /dev/null 2>&1
  $ SUMMARY=$(/bin/ls .turbo/runs/*.json | head -n1)
  $ getSummaryTaskId $SUMMARY "my-app#build" | jq .dependencies
  [
    "another#build",
    "util#build"
  ]
