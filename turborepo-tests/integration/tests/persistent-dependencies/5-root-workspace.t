# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/5-root-workspace

# WorkspaceGraph: no package dependencies
#
# Task Graph:
# build
# └── //#dev
#
# With this workspace graph, that means:
#
# app-a#build
# └── //#dev
#
  $ ${TURBO} run build
    x Invalid task configuration
    `->   x "//#dev" is a persistent task, "app-a#build" cannot depend on it
           ,-[turbo.json:5:21]
         4 |     "build": {
         5 |       "dependsOn": ["//#dev"],
           :                     ^^^^|^^^
           :                         `-- persistent task
         6 |       "persistent": true
           `----
  
  [1]
