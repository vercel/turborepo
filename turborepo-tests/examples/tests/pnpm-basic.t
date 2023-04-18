  $ . ${TESTDIR}/setup.sh basic pnpm
# run twice and make sure it works
  $ ${TURBO} build lint > /dev/null 2>&1
  $ ${TURBO} build lint > /dev/null 2>&1
  $ git diff
