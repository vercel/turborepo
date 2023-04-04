  $ . ${TESTDIR}/setup.sh with-npm npm
# run twice and make sure it works
  $ npm run build lint 2>&1 > /dev/null
  $ npm run build lint 2>&1 > /dev/null
  $ git diff
