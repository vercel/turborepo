Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/../../helpers/mock_turbo_config.sh

Logout while logged in
  $ ${TURBO} logout
  >>> Logged out

Logout while logged out
  $ ${TURBO} logout
  >>> Logged out

