# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/6-topological-unimplemented

# WorkspaceGraph
# - app-a depends on pkg-a
#
# Task Graph:
# dev
# └── ^dev
#
# With this workspace graph, that means:
#
# app-a#dev
# └── pkg-a#dev (but this isn't implemented)
# 
  $ ${TURBO} run dev
  \xe2\x80\xa2 Packages in scope: app-a, pkg-a (esc)
  \xe2\x80\xa2 Running dev in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:dev: cache miss, executing 7def3a3e7f1235de
  app-a:dev: 
  app-a:dev: > dev
  app-a:dev: > echo dev-app-a
  app-a:dev: 
  app-a:dev: dev-app-a
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  
