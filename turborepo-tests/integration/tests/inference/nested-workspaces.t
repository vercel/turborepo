Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/nested_workspaces_setup.sh $(pwd)/nested_workspaces

  $ cd $TARGET_DIR/outer && ${TURBO} run build --filter=nothing -vv 1> OUTER 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer" OUTER
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER

  $ cd $TARGET_DIR/outer/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer" OUTER_APPS
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER_APPS

  $ cd $TARGET_DIR/outer/inner && ${TURBO} run build --filter=nothing -vv 1> OUTER_INNER 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer[\/\\]inner" OUTER_INNER
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER_INNER

  $ cd $TARGET_DIR/outer/inner/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_INNER_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer[\/\\]inner" OUTER_INNER_APPS
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER_INNER_APPS

Locate a repository with no turbo.json. We'll get the right root, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer[\/\\]inner-no-turbo" INNER_NO_TURBO
  $ grep --quiet "x Could not find turbo.json." INNER_NO_TURBO
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" INNER_NO_TURBO

Locate a repository with no turbo.json. We'll get the right root and inference directory, but there's nothing to run
  $ cd $TARGET_DIR/outer/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer[\/\\]inner-no-turbo" INNER_NO_TURBO_APPS
  $ grep --quiet "x Could not find turbo.json." INNER_NO_TURBO_APPS
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" INNER_NO_TURBO_APPS

  $ cd $TARGET_DIR/outer-no-turbo && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo" OUTER_NO_TURBO
  $ grep --quiet "x Could not find turbo.json." OUTER_NO_TURBO
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" OUTER_NO_TURBO

  $ cd $TARGET_DIR/outer-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo" OUTER_NO_TURBO_APPS
  $ grep --quiet "x Could not find turbo.json." OUTER_NO_TURBO_APPS
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" OUTER_NO_TURBO_APPS

  $ cd $TARGET_DIR/outer-no-turbo/inner && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_INNER 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo[\/\\]inner" OUTER_NO_TURBO_INNER
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER_NO_TURBO_INNER

  $ cd $TARGET_DIR/outer-no-turbo/inner/apps && ${TURBO} run build --filter=nothing -vv 1> OUTER_NO_TURBO_INNER_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo[\/\\]inner" OUTER_NO_TURBO_INNER_APPS
  $ grep --quiet "No package found with name 'nothing' in workspace" OUTER_NO_TURBO_INNER_APPS

  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo[\/\\]inner-no-turbo" INNER_NO_TURBO
  $ grep --quiet "x Could not find turbo.json." INNER_NO_TURBO
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" INNER_NO_TURBO

  $ cd $TARGET_DIR/outer-no-turbo/inner-no-turbo/apps && ${TURBO} run build --filter=nothing -vv 1> INNER_NO_TURBO_APPS 2>&1
  [1]
  $ grep --quiet -E "Repository Root: .*[\/\\]nested_workspaces[\/\\]outer-no-turbo[\/\\]inner-no-turbo" INNER_NO_TURBO_APPS
  $ grep --quiet "x Could not find turbo.json." INNER_NO_TURBO_APPS
  $ grep --quiet "| Follow directions at https://turbo.build/repo/docs to create one" INNER_NO_TURBO_APPS

