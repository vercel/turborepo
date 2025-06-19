# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/3-workspace-specific

# Workspace Graph:
# - app-a depends on pkg-a
#
# Task Graph:
# build
# └── workspace-b#dev
#
# With this workspace graph, that means:
#
# app-a#build
# └── pkg-a#dev
# pkg-a#build
# └── pkg-a#dev
#
# The regex match is liberal, because the build task from either workspace can throw the error
  $ ${TURBO} run build
    x Invalid task configuration
    |->   x "pkg-a#dev" is a persistent task, "app-a#build" cannot depend on it
    |      ,-[turbo.json:5:21]
    |    4 |     "build": {
    |    5 |       "dependsOn": ["pkg-a#dev"]
    |      :                     ^^^^^|^^^^^
    |      :                          `-- persistent task
    |    6 |     },
    |      `----
    `->   x "pkg-a#dev" is a persistent task, "pkg-a#build" cannot depend on it
           ,-[turbo.json:5:21]
         4 |     "build": {
         5 |       "dependsOn": ["pkg-a#dev"]
           :                     ^^^^^|^^^^^
           :                          `-- persistent task
         6 |     },
           `----
  
  [1]
