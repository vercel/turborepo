Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/logged_in.sh

Logout while logged in
  $ ${TURBO} logout
  >>> Logged out

Logout while logged out
  $ ${TURBO} logout
  >>> Logged out

