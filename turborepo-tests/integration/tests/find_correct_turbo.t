  $ . ${TESTDIR}/../../helpers/setup.sh

Make sure exit code is 2 when no args are passed
  $ CURR=$(${TURBO} --cwd ${TESTDIR}/../.. bin)
  $ diff <(readlink -f ${TURBO}) <(readlink -f ${CURR})
  /bin/sh: line 6: syntax error near unexpected token `('
  /bin/sh: line 6: `diff <(readlink -f ${TURBO}) <(readlink -f ${CURR})' (no-eol)

