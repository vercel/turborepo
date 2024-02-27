Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Start daemon
  $ ${TURBO} daemon start
  \xe2\x9c\x93 daemon is running (esc)

Confirm daemon status
  $ ${TURBO} daemon status
  \xe2\x9c\x93 daemon is running (esc)
  log file: .* (re)
  uptime: .* (re)
  pid file: .* (re)
  socket file: .* (re)

Subscribe to a package (we don't care about the output here
because the daemon might not compute the hash in time for the deadline)
  $ sleep 5
  $ ${TURBO} daemon hash my-app#build
  my-app#build: 1618c35ab3d16bfb