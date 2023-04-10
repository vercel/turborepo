# Setup
  $ . ${TESTDIR}/../_helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) persistent_dependencies/3-workspace-specific

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
   ERROR  run failed: error preparing engine: Invalid persistent task configuration:
  "pkg-a#dev" is a persistent task, .*-a#build" cannot depend on it (re)
  Turbo error: error preparing engine: Invalid persistent task configuration:
  "pkg-a#dev" is a persistent task, .*-a#build" cannot depend on it (re)
  [1]
