  $ . ${TESTDIR}/setup.sh with-yarn npm
# run twice and make sure it works
  $ npm run build lint 2>&1 > /dev/null
  $ npm run build lint 2>&1 > /dev/null  
  $ git diff
