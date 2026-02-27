Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Make sure exit code is 1 and help text is printed when no args are passed
  $ ${TURBO} 2> out.txt
  [1]
  $ head -n1 out.txt
  The build system that makes ship happen

Run without any tasks, get a list of potential tasks to run
  $ ${TURBO} run
  No tasks provided, here are some potential ones
  
    build
      my-app, util
    maybefails
      my-app, util
    dev
      another
  [1]

Run again with a filter and get only the packages that match
  $ ${TURBO} run --filter my-app
  No tasks provided, here are some potential ones
  
    build
      my-app
    maybefails
      my-app
  [1]

Watch without any tasks, get a list of potential tasks to watch
  $ ${TURBO} watch
  No tasks provided, here are some potential ones
  
    build
      my-app, util
    maybefails
      my-app, util
    dev
      another
  [1]

Run again with an environment variable that corresponds to a run argument and assert that
we get the full help output.
  $ TURBO_LOG_ORDER=stream ${TURBO} 2> out.txt
  [1]
  $ cat out.txt | head -n1
  The build system that makes ship happen

Initialize a new monorepo
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config > /dev/null 2>&1

  $ ${TURBO} run
  No tasks provided, here are some potential ones
  
    build
      invalid-config, my-app, util
    maybefails
      my-app, util
    add-keys-task
      add-keys
    add-keys-underlying-task
      add-keys
    added-task
      add-tasks
    cached-task-1
      cached
    cached-task-2
      cached
    cached-task-3
      cached
    cached-task-4
      missing-workspace-config
    config-change-task
      config-change
    cross-workspace-task
      cross-workspace
    cross-workspace-underlying-task
      blank-pkg
    dev
      another
    missing-workspace-config-task
      missing-workspace-config
    missing-workspace-config-task-with-deps
      missing-workspace-config
    missing-workspace-config-underlying-task
      missing-workspace-config
    missing-workspace-config-underlying-topo-task
      blank-pkg
    omit-keys-task
      omit-keys
    omit-keys-task-with-deps
      omit-keys
    omit-keys-underlying-task
      omit-keys
    omit-keys-underlying-topo-task
      blank-pkg
    override-values-task
      override-values
    override-values-task-with-deps
      override-values
    override-values-task-with-deps-2
      override-values
    override-values-underlying-task
      override-values
    override-values-underlying-topo-task
      blank-pkg
    persistent-task-1
      persistent
    persistent-task-1-parent
      persistent
    persistent-task-2
      persistent
    persistent-task-2-parent
      persistent
    persistent-task-3
      persistent
    persistent-task-3-parent
      persistent
    persistent-task-4
      persistent
    persistent-task-4-parent
      persistent
    trailing-comma
      bad-json
  [1]

  $ ${TURBO} watch
  No tasks provided, here are some potential ones
  
    build
      invalid-config, my-app, util
    maybefails
      my-app, util
    add-keys-task
      add-keys
    add-keys-underlying-task
      add-keys
    added-task
      add-tasks
    cached-task-1
      cached
    cached-task-2
      cached
    cached-task-3
      cached
    cached-task-4
      missing-workspace-config
    config-change-task
      config-change
    cross-workspace-task
      cross-workspace
    cross-workspace-underlying-task
      blank-pkg
    dev
      another
    missing-workspace-config-task
      missing-workspace-config
    missing-workspace-config-task-with-deps
      missing-workspace-config
    missing-workspace-config-underlying-task
      missing-workspace-config
    missing-workspace-config-underlying-topo-task
      blank-pkg
    omit-keys-task
      omit-keys
    omit-keys-task-with-deps
      omit-keys
    omit-keys-underlying-task
      omit-keys
    omit-keys-underlying-topo-task
      blank-pkg
    override-values-task
      override-values
    override-values-task-with-deps
      override-values
    override-values-task-with-deps-2
      override-values
    override-values-underlying-task
      override-values
    override-values-underlying-topo-task
      blank-pkg
    persistent-task-1
      persistent
    persistent-task-1-parent
      persistent
    persistent-task-2
      persistent
    persistent-task-2-parent
      persistent
    persistent-task-3
      persistent
    persistent-task-3-parent
      persistent
    persistent-task-4
      persistent
    persistent-task-4-parent
      persistent
    trailing-comma
      bad-json
  [1]
