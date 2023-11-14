Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/nested_workspaces_setup.sh $(pwd)/nested_workspaces

  $ cd $TARGET_DIR/outer && ${TURBO} run build --filter=nothing -vv 1> OUTER 2>&1
  $ cat OUTER | grep --only-match -E "Repository Root: .*/nested_workspaces/outer"
  Repository Root: .*/nested_workspaces/outer (re)
  $ cat OUTER | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/outer/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_APPS 2>&1
  $ cat OUTER_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer"
  Repository Root: .*/nested_workspaces/outer (re)
  $ cat OUTER_APPS | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/outer/inner && ${TURBO} run build --filter=nothing -vv 1> OUTER_INNER 2>&1
  $ cat OUTER_INNER | grep --only-match -E "Repository Root: .*/nested_workspaces/outer/inner"
  Repository Root: .*/nested_workspaces/outer/inner (re)
  $ cat OUTER_INNER | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/outer/inner/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_INNER_APPS 2>&1
  $ cat OUTER_INNER_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer/inner"
  Repository Root: .*/nested_workspaces/outer/inner (re)
  $ cat OUTER_INNER_APPS | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

Locate a repository with no turbo.json. We'll get the right root, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO 2>&1
  [1]
  $ cat INNER_NO_TURBO | grep --only-match -E "Repository Root: .*/nested_workspaces/outer/inner-no-turbo"
  Repository Root: .*/nested_workspaces/outer/inner-no-turbo (re)
  $ cat INNER_NO_TURBO | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

Locate a repository with no turbo.json. We'll get the right root and inference directory, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO_APPS 2>&1
  [1]
  $ cat INNER_NO_TURBO_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer/inner-no-turbo"
  Repository Root: .*/nested_workspaces/outer/inner-no-turbo (re)
  $ cat INNER_NO_TURBO_APPS | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

  $ cd $TARGET_DIR/outer-no-turbo && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO 2>&1
  [1]
  $ cat OUTER_NO_TURBO | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo"
  Repository Root: .*/nested_workspaces/outer-no-turbo (re)
  $ cat OUTER_NO_TURBO | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

  $ cd $TARGET_DIR/outer-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_APPS 2>&1
  [1]
  $ cat OUTER_NO_TURBO_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo"
  Repository Root: .*/nested_workspaces/outer-no-turbo (re)
  $ cat OUTER_NO_TURBO_APPS | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

  $ cd $TARGET_DIR/outer-no-turbo/inner && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_INNER 2>&1
  $ cat OUTER_NO_TURBO_INNER | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo/inner"
  Repository Root: .*/nested_workspaces/outer-no-turbo/inner (re)
  $ cat OUTER_NO_TURBO_INNER | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/outer-no-turbo/inner/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_INNER_APPS 2>&1
  $ cat OUTER_NO_TURBO_INNER_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo/inner"
  Repository Root: .*/nested_workspaces/outer-no-turbo/inner (re)
  $ cat OUTER_NO_TURBO_INNER_APPS | grep --only-match "No tasks were executed as part of this run."
  No tasks were executed as part of this run.

  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO 2>&1
  [1]
  $ cat INNER_NO_TURBO | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo"
  Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo (re)
  $ cat INNER_NO_TURBO | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO_APPS 2>&1
  [1]
  $ cat INNER_NO_TURBO_APPS | grep --only-match -E "Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo"
  Repository Root: .*/nested_workspaces/outer-no-turbo/inner-no-turbo (re)
  $ cat INNER_NO_TURBO_APPS | grep --only-match "Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one"
  Could not find turbo.json. Follow directions at https://turbo.build/repo/docs to create one

