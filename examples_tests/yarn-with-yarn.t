  $ . ${TESTDIR}/setup.sh with-yarn yarn
# run twice and make sure it works
  $ yarn turbo build lint > /dev/null
  $ yarn turbo build lint > /dev/null
  $ git diff
