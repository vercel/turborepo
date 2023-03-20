  $ . ${TESTDIR}/setup.sh with-yarn yarn
# A single comment
# run twice and make sure it works
  $ yarn turbo build lint > /dev/null
  $ yarn turbo build lint > /dev/null
  $ git diff
