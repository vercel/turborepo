Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd)

Run info
  $ ${TURBO} info
  3 packages found in workspace
  
  - another packages/another/package.json
  - my-app apps/my-app/package.json
  - util packages/util/package.json


Run info on package `another`
  $ ${TURBO} info another
  another depends on:
  - root

Run info on package `my-app`
  $ ${TURBO} info my-app
  my-app depends on:
  - root
  - util
