# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/9-cross-workspace-nested

// Workspace Graph
// - No workspace dependencies
// 
// Task Graph:
//
// workspace-a#build
// └── workspace-b#build
// 		 └── workspace-c#build
// 		 		 └── workspace-z#dev // this one is persistent
//
  $ ${TURBO} run build
    x invalid task configuration
  
  Error:   x "app-z#dev" is a persistent task, "app-c#build" cannot depend on it
      ,-[turbo.json:12:1]
   12 |     "app-c#build": {
   13 |       "dependsOn": ["app-z#dev"]
      :                     ^^^^^|^^^^^
      :                          `-- persistent task
   14 |     },
      `----
  
  [1]
