Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run info
  $ ${TURBO} ls
  3 packages (npm)
  
    another packages[\/\\]another (re)
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run ls with filter
  $ ${TURBO} ls -F my-app...
  2 packages (npm)
  
    my-app apps[\/\\]my-app (re)
    util packages[\/\\]util (re)

Run ls on package `another`
  $ ${TURBO} ls another
  packages[/\\]another  (re)
  another depends on: <no packages>
  
  tasks:
    dev: echo building
  

Run ls on package `my-app`
  $ ${TURBO} ls my-app
  apps[\/\\]my-app  (re)
  my-app depends on: util
  
  tasks:
    build: echo building
    maybefails: exit 4
  
