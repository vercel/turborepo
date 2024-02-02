# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh persistent_dependencies/7-topological-nested

# WorkspaceGraph
# - app-a depends on pkg-a
# - pkg-a depends on pkg-b
#
# Task Graph:
# dev
# └── ^dev
#
# That means:
#
# app-a#dev
# └── pkg-a#dev (this isn't implemented)
# 		 └── pkg-b#dev

# Note: This error is interesting because pka-b doesn't implement a dev task, so saying that pkg-a
# shouldn't depend on it is weird. This is partly unavoidable, but partly debatable about what the
# error message should say. Leaving as-is so we don't have to implement special casing logic to handle
# this case.
  $ ${TURBO} run dev
    x error preparing engine: Invalid persistent task configuration:
    | "pkg-b#dev" is a persistent task, "pkg-a#dev" cannot depend on it
  
  [1]
