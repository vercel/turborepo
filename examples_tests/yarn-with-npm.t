  $ . ${TESTDIR}/setup.sh with-npm yarn
# run twice and make sure it works
  $ yarn build lint 2>&1 > /dev/null
  $ yarn build lint 2>&1 > /dev/null
  $ git diff
