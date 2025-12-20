# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/1-topological

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
    x Invalid task configuration
    `->   x "pkg-a#dev" is a persistent task, "app-a#dev" cannot depend on it
           ,-[turbo.json:5:21]
         4 |     "dev": {
         5 |       "dependsOn": ["^dev"],
           :                     ^^^|^^
           :                        `-- persistent task
         6 |       "persistent": true
           `----
  
  [1]
