Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

TURBO_LOG_VERBOSITY should be respected
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status > tmp.log 2>&1
  [1]
  $ grep --quiet -E "\[DEBUG].*" tmp.log
  $ tail -n1 tmp.log | grep --quiet "Turbo error: unable to connect: daemon is not running"

-v flag overrides TURBO_LOG_VERBOSITY global setting
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} daemon status -v > tmp.log 2>&1
  [1]
  $ grep --quiet -E "\[DEBUG].*" tmp.log # DEBUG logs not found
  [1]
  $ tail -n1 tmp.log | grep --quiet "Turbo error: unable to connect: daemon is not running"

verbosity doesn't override TURBO_LOG_VERBOSITY package settings
  $ TURBO_LOG_VERBOSITY=turborepo_lib=debug ${TURBO} daemon status -v > tmp.log 2>&1
  [1]
  $ grep --quiet -E "\[DEBUG].*" tmp.log
  $ tail -n1 tmp.log | grep --quiet "Turbo error: unable to connect: daemon is not running"
