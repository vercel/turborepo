  $ . ${TESTDIR}/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  $ bash -c 'diff <(readlink -f ${TURBO}) <(readlink -f ${CURR})'
