  $ . ${TESTDIR}/setup.sh with-gatsby pnpm
# run twice and make sure it works
  $ pnpm run build lint > /dev/null
  $ pnpm run build lint > /dev/null
  $ git diff
