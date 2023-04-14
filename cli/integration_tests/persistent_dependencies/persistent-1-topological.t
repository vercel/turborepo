# Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) persistent_dependencies/1-topological

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
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  "pkg-a#dev" is a persistent task, "app-a#dev" cannot depend on it
  Turbo error: error preparing engine: Invalid persistent task configuration:
  "pkg-a#dev" is a persistent task, "app-a#dev" cannot depend on it
  [1]
