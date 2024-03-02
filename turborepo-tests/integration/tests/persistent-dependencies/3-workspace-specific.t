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
    x invalid persistent task configuration
  
  Error:   x "pkg-a#dev" is a persistent task, "((pkg-a)|(app-a))#build" cannot depend on it (re)
     ,-[turbo.json:4:1]
   4 |     "build": {
   5 |       "dependsOn": ["pkg-a#dev"]
     :                     ^^^^^|^^^^^
     :                          `-- persistent task
   6 |     },
     `----
  Error:   x "pkg-a#dev" is a persistent task, "((pkg-a)|(app-a))#build" cannot depend on it (re)
     ,-[turbo.json:4:1]
   4 |     "build": {
   5 |       "dependsOn": ["pkg-a#dev"]
     :                     ^^^^^|^^^^^
     :                          `-- persistent task
   6 |     },
     `----
  
  [1]
