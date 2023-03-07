  $ . ${TESTDIR}/setup.sh non-monorepo yarn
# run twice and make sure it works
  $ npx turbo build lint > /dev/null
  $ npx turbo build lint > /dev/null
  $ git diff
