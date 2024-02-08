# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/8-topological-with-extra

// WorkspaceGraph:
// - app-a depends on pkg-a
//  -pkg-a depends on pkg-b
//  -pkg-b depends on pkg-z
//
// TaskGraph:
// build
// └── ^build
// pkg-b#build
// └── pkg-z#dev
//
// With this workspace graph, that means:
//
// workspace-a#build
// └── workspace-b#build
// 		 └── workspace-c#build
// 		 		 └── workspace-z#dev	// this one is persistent
  $ ${TURBO} run build
    x error preparing engine: Invalid persistent task configuration:
    | "pkg-z#dev" is a persistent task, "pkg-b#build" cannot depend on it
  
  [1]
