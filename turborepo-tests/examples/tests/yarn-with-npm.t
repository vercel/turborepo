  $ . ${TESTDIR}/setup.sh with-npm yarn
# run twice and make sure it works
  $ yarn turbo build lint > /dev/null 2>&1
  $ yarn turbo build lint > /dev/null 2>&1
  $ git diff
