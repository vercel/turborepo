Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run info
  $ ${TURBO} ls
   WARNING  ls command is experimental and may change in the future
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run ls with filter
  $ ${TURBO} ls -F my-app...
   WARNING  ls command is experimental and may change in the future
  2 packages (npm)
  
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run ls on package `another`
  $ ${TURBO} ls another
   WARNING  ls command is experimental and may change in the future
  packages/another 
  another depends on: <no packages>
  
  tasks:
    dev: echo building
  

Run ls on package `my-app`
  $ ${TURBO} ls my-app
   WARNING  ls command is experimental and may change in the future
  apps/my-app 
  my-app depends on: util
  
  tasks:
    build: echo building
    maybefails: exit 4
  
