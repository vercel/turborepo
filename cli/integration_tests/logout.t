Setup
  $ . ${TESTDIR}/setup.sh
  $ . ${TESTDIR}/logged_in.sh

Logout while logged in
  $ ${SHIM} logout
  >>> Logged out

Logout while logged out
  $ ${SHIM} logout
  >>> Logged out

