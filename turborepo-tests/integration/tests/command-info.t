Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run info
  $ ${TURBO} info
  You are not logged in
  3 packages found in workspace
  
  - another packages(\/|\\)another (re)
  - my-app apps(\/|\\)my-app (re)
  - util packages(\/|\\)util (re)


Run info on package `another`
  $ ${TURBO} info another
  another depends on:
  - root

Run info on package `my-app`
  $ ${TURBO} info my-app
  my-app depends on:
  - root
  - util
