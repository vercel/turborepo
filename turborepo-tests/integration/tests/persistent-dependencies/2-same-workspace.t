# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/2-same-workspace

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
    x error preparing engine: Invalid persistent task configuration:
    | "app-a#dev" is a persistent task, "app-a#build" cannot depend on it
  
  [1]
