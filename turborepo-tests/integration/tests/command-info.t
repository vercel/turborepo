Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

Run info
  $ ${TURBO} info
  You are not logged in
  3 packages found in workspace
  
  - another packages/another
  - my-app apps/my-app
  - util packages/util


Run info on package `another`
  $ ${TURBO} info another
  another depends on:
  - root

Run info on package `my-app`
  $ ${TURBO} info my-app
  my-app depends on:
  - root
  - util
