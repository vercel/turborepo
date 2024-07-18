Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run info
  $ ${TURBO} ls
   WARNING  ls command is experimental and may change in the future
  3 packages
  
    another packages/another
    my-app apps/my-app
    util packages/util

Run info with filter
  $ ${TURBO} ls -F my-app...
   WARNING  ls command is experimental and may change in the future
  2 packages
  
    my-app apps/my-app
    util packages/util

Run info on package `another`
  $ ${TURBO} ls another
   WARNING  ls command is experimental and may change in the future
  another depends on: <no packages>
  
  tasks: <no tasks>
  

Run info on package `my-app`
  $ ${TURBO} ls my-app
   WARNING  ls command is experimental and may change in the future
  my-app depends on: util
  
  tasks:
    build: echo building
    maybefails: exit 4
  
