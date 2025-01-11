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
    x invalid task configuration
  
  Error:   x "app-a#dev" is a persistent task, "app-a#build" cannot depend on it
     ,-[turbo.json:4:1]
   4 |     "build": {
   5 |       "dependsOn": ["dev"]
     :                     ^^|^^
     :                       `-- persistent task
   6 |     },
     `----
  
  [1]
