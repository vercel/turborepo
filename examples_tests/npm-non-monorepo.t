  $ . ${TESTDIR}/setup.sh non-monorepo npm
# run twice and make sure it works
  $ npx turbo build lint 2>&1 > /dev/null
  $ npx turbo build lint 2>&1 > /dev/null
  $ git diff
