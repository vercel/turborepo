Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Running a build with the daemon forced should run in daemon mode
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} run build --daemon > tmp.log 2>&1
  $ grep --quiet -E "turborepo_lib::run: running in daemon mode" tmp.log
  $ grep -E "turborepo_lib::process::child: child process exited normally" tmp.log
  (.+) turborepo_lib::process::child: child process exited normally (re)
  (.+) turborepo_lib::process::child: child process exited normally (re)

On some platforms we can't perform cleanup if the daemon is running, so stop it
  $ ${TURBO} daemon stop
