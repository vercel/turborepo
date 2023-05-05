Setup
  $ . ${TESTDIR}/../../helpers/setup.sh

The daemon exits when there is a stale pid file
  $ ${TURBO} daemon & sleep 1 && kill $! && ${TURBO} daemon
  WARN stale pid file at ".+" (re)
  ERROR error opening socket: pidlock error: lock exists at ".+", please remove it (re)
  /bin/bash: line 4: .+ (re)
  [1]

A message is printed when the daemon is running already
  $ rm -r ${TMPDIR}/turbod; ${TURBO} daemon & (export PID=$!; sleep 1 && ${TURBO} daemon && kill $PID && kill $PID && kill $PID)
  WARN daemon already running
