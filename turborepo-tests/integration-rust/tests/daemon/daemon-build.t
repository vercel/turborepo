Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Running a build with the daemon forced should run in daemon mode
Also checks that the daemon workspace discovery handles cold boot
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} run build --daemon > tmp.log 2>&1
  $ grep --quiet -E "turborepo_lib::run: running in daemon mode" tmp.log
Also check we only make one request to the daemon
  $ grep -E "turborepo_lib::run::package_discovery: discovering packages using daemon" tmp.log
  (.+) turborepo_lib::run::package_discovery: discovering packages using daemon (re)
Two tasks are run
  $ grep -E "turborepo_lib::process::child: child process exited normally" tmp.log
  (.+) turborepo_lib::process::child: child process exited normally (re)
  (.+) turborepo_lib::process::child: child process exited normally (re)
Create a new package
  $ cp -r apps/my-app apps/my-app2
  $ sed -i'.bak' -e 's/my-app/my-app2/g' apps/my-app2/package.json
Running daemon-based package discovery discovers the new package and run only that
  $ TURBO_LOG_VERBOSITY=debug ${TURBO} run build --daemon > tmp.log 2>&1
  $ grep --quiet -E "turborepo_lib::run: running in daemon mode" tmp.log
Check we only make one request to the daemon
  $ grep -E "turborepo_lib::run::package_discovery: discovering packages using daemon" tmp.log
  (.+) turborepo_lib::run::package_discovery: discovering packages using daemon (re)
Run that one task
  $ grep -E "turborepo_lib::process::child: child process exited normally" tmp.log
  (.+) turborepo_lib::process::child: child process exited normally (re)
On some platforms we can't perform cleanup if the daemon is running, so stop it
  $ ${TURBO} daemon stop
