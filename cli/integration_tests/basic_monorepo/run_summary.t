Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)
  $ rm -rf .turbo/runs
  $ TURBO_RUN_SUMMARY=true ${TURBO} run build > /dev/null
  $ ls .turbo/runs/*.json | wc -l
         1

# Without env var, no summary file is generated
  $ rm -rf .turbo/runs
  $ ${TURBO} run build > /dev/null
  $ find -d .turbo/runs
  .* No such file or directory (re)
  [1]
