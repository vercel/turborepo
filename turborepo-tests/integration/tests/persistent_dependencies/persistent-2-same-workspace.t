# Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) persistent_dependencies/2-same-workspace

// WorkspaceGraph: no package dependencies
// 
// Task Graph:
// build
// └── dev
//
// That means:
//
// app-a#build
// └── app-a#dev
//
  $ ${TURBO} run build
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  "app-a#dev" is a persistent task, "app-a#build" cannot depend on it
  Turbo error: error preparing engine: Invalid persistent task configuration:
  "app-a#dev" is a persistent task, "app-a#build" cannot depend on it
  [1]
