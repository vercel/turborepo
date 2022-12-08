  $ . ${TESTDIR}/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  $ if [[ "${TURBO}" != "${CURR}" ]]; then echo "Expected ${TURBO}, Got ${CURR}"; else echo "Got correct binary"; fi
