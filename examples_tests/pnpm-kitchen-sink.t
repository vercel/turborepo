  $ . ${TESTDIR}/setup.sh kitchen-sink pnpm
# run twice and make sure it works
  $ pnpm run build lint > /dev/null
  $ pnpm run build lint > /dev/null
  $ git diff
