  $ . ${TESTDIR}/setup.sh with-yarn yarn
# run twice and make sure it works
  $ yarn turbo build lint 2>&1 > /dev/null
  $ yarn turbo build lint 2>&1 > /dev/null
  $ git diff
