  $ . ${TESTDIR}/setup.sh with-npm npm
# run twice and make sure it works
  $ npm run build lint > /dev/null 2>&1
  $ npm run build lint > /dev/null 2>&1
  $ git diff
