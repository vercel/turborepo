Ignore Yarn warning that it found a package-lock.json
  $ . ${TESTDIR}/setup.sh with-npm yarn 2> /dev/null
# run twice and make sure it works
  $ yarn build lint > /dev/null
  $ yarn build lint > /dev/null
  $ git diff
