  $ . ${TESTDIR}/setup.sh with-yarn npm
# run twice and make sure it works
  $ npm run build lint > /dev/null
  $ npm run build lint > /dev/null  
  $ git diff
