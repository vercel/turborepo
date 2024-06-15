  $ . ${TESTDIR}/../../helpers/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  $ diff --strip-trailing-cr <(readlink -f ${TURBO}) <(readlink -f ${CURR})

