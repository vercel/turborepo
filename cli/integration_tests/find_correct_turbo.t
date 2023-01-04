  $ . ${TESTDIR}/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  $ diff <(readlink -f ${TURBO}) <(readlink -f ${CURR})
