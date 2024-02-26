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

Try hashing a package
  $ ${TURBO} daemon hash my-app#build
  my-app#build: 1618c35ab3d16bfb
