# Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) 1-topological

// Workspace Graph
// - app-a depends on pkg-a
// 
// Make this Task Graph:
// dev
// └── ^dev
//
// With this workspace graph, that means:
//
// app-a#dev
// └── pkg-a#dev
  $ ${TURBO} run dev
   ERROR  run failed: error preparing engine: Invalid persistent task dependency:
  "pkg-a#dev" is a persistent task, "app-a#dev" cannot depend on it
  Turbo error: error preparing engine: Invalid persistent task dependency:
  "pkg-a#dev" is a persistent task, "app-a#dev" cannot depend on it
  [1]
