Setup
  $ . ${TESTDIR}/setup.sh
  $ . ${TESTDIR}/logged_in.sh

Logout while logged in
  $ ${TURBO} logout
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  >>> Logged out

Logout while logged out
  $ ${TURBO} logout
  Repository inference failed: Unable to find `turbo.json` or `package.json` in current path
  Running command as global turbo
  >>> Logged out

