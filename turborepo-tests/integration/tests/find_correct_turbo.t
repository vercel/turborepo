  $ . ${TESTDIR}/../../helpers/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  $ (readlink -f ${TURBO}) > turbo
  $ (readlink -f ${CURR}) > curr
  $ diff turbo curr

