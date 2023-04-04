  $ . ${TESTDIR}/setup.sh with-gatsby pnpm
# run twice and make sure it works
  $ pnpm run build lint 2>&1 > /dev/null
  $ pnpm run build lint 2>&1 > /dev/null
  $ git diff
