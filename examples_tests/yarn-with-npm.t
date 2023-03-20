  $ . ${TESTDIR}/setup.sh with-npm yarn
# run twice and make sure it works
  $ yarn build lint > /dev/null
  $ yarn build lint > /dev/null
  $ git diff
