Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The missing-workspace-config-task-with-deps configures dependsOn in the root turbo.json.
# The workspace does not have a turbo.json config. This test checks that both regular dependencies
# and Topological dependencies are retained from the root config.

# 1. First run, assert that dependet tasks run `dependsOn`
  $ ${TURBO} run missing-workspace-config-task-with-deps --filter=missing-workspace-config > tmp.log
# Validate in pieces. `omit-key` task has two dependsOn values, and those tasks
# can run in non-deterministic order. So we need to validate the logs in the pieces.
  $ cat tmp.log | grep "in scope" -A 2
  \xe2\x80\xa2 Packages in scope: missing-workspace-config (esc)
  \xe2\x80\xa2 Running missing-workspace-config-task-with-deps in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)

  $ cat tmp.log | grep "missing-workspace-config:missing-workspace-config-task-with-deps"
  missing-workspace-config:missing-workspace-config-task-with-deps: cache miss, executing cb5a7b7c7ef29b91
  missing-workspace-config:missing-workspace-config-task-with-deps: 
  missing-workspace-config:missing-workspace-config-task-with-deps: > missing-workspace-config-task-with-deps
  missing-workspace-config:missing-workspace-config-task-with-deps: > echo running-missing-workspace-config-task-with-deps > out/foo.min.txt
  missing-workspace-config:missing-workspace-config-task-with-deps: 

  $ cat tmp.log | grep "missing-workspace-config:missing-workspace-config-underlying-task"
  missing-workspace-config:missing-workspace-config-underlying-task: cache miss, executing 26878c99d9f1f2ad
  missing-workspace-config:missing-workspace-config-underlying-task: 
  missing-workspace-config:missing-workspace-config-underlying-task: > missing-workspace-config-underlying-task
  missing-workspace-config:missing-workspace-config-underlying-task: > echo running-missing-workspace-config-underlying-task
  missing-workspace-config:missing-workspace-config-underlying-task: 
  missing-workspace-config:missing-workspace-config-underlying-task: running-missing-workspace-config-underlying-task

  $ cat tmp.log | grep "blank-pkg:missing-workspace-config-underlying-topo-task"
  blank-pkg:missing-workspace-config-underlying-topo-task: cache miss, executing 86d7535cfbce352a
  blank-pkg:missing-workspace-config-underlying-topo-task: 
  blank-pkg:missing-workspace-config-underlying-topo-task: > missing-workspace-config-underlying-topo-task
  blank-pkg:missing-workspace-config-underlying-topo-task: > echo missing-workspace-config-underlying-topo-task from blank-pkg
  blank-pkg:missing-workspace-config-underlying-topo-task: 
  blank-pkg:missing-workspace-config-underlying-topo-task: missing-workspace-config-underlying-topo-task from blank-pkg

  $ cat tmp.log | grep "Tasks:" -A 2
   Tasks:    3 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
