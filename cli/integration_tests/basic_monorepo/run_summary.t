Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build > /dev/null
# no output, just check for 0 status code
  $ test -d .turbo/runs
  $ ls .turbo/runs/*.json | wc -l
  \s*1 (re)

# Without env var, no summary file is generated
  $ rm -rf .turbo/runs
  $ ${TURBO} run build > /dev/null
# validate with exit code so the test works on macOS and linux
  $ test -d .turbo/runs
  [1]
