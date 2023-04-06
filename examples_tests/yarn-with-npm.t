  $ . ${TESTDIR}/setup.sh with-npm yarn
# run twice and make sure it works
  $ yarn build lint > /dev/null 2>&1
  $ yarn build lint > /dev/null 2>&1
  $ git diff
