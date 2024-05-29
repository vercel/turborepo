Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

# The omit-keys-task-with-deps configures dependsOn. The workspace config
# defines the task, but does not override anything. This test checks
# that both regular dependencies and Topological dependencies are retained
# from the root config.

# 1. First run, assert for `dependsOn` and `outputs` keys
  $ ${TURBO} run omit-keys-task-with-deps --filter=omit-keys > tmp.log
# Validate in pieces. `omit-key` task has two dependsOn values, and those tasks
# can run in non-deterministic order. So we need to validatte the logs in pieces.
  $ cat tmp.log | grep "in scope" -A 1
  \xe2\x80\xa2 Packages in scope: omit-keys (esc)
  \xe2\x80\xa2 Running omit-keys-task-with-deps in 1 packages (esc)

  $ cat tmp.log | grep "omit-keys:omit-keys-task-with-deps"
  omit-keys:omit-keys-task-with-deps: cache miss, executing bbb54c4130d16663
  omit-keys:omit-keys-task-with-deps: 
  omit-keys:omit-keys-task-with-deps: > omit-keys-task-with-deps
  omit-keys:omit-keys-task-with-deps: > echo running-omit-keys-task-with-deps > out/foo.min.txt
  omit-keys:omit-keys-task-with-deps: 

  $ cat tmp.log | grep "omit-keys:omit-keys-underlying-task"
  omit-keys:omit-keys-underlying-task: cache miss, executing 1aa42011f41a10f1
  omit-keys:omit-keys-underlying-task: 
  omit-keys:omit-keys-underlying-task: > omit-keys-underlying-task
  omit-keys:omit-keys-underlying-task: > echo running-omit-keys-underlying-task
  omit-keys:omit-keys-underlying-task: 
  omit-keys:omit-keys-underlying-task: running-omit-keys-underlying-task

  $ cat tmp.log | grep "blank-pkg:omit-keys-underlying-topo-task"
  blank-pkg:omit-keys-underlying-topo-task: cache miss, executing 4510d84de8b1d9b7
  blank-pkg:omit-keys-underlying-topo-task: 
  blank-pkg:omit-keys-underlying-topo-task: > omit-keys-underlying-topo-task
  blank-pkg:omit-keys-underlying-topo-task: > echo omit-keys-underlying-topo-task from blank-pkg
  blank-pkg:omit-keys-underlying-topo-task: 
  blank-pkg:omit-keys-underlying-topo-task: omit-keys-underlying-topo-task from blank-pkg

  $ cat tmp.log | grep "Tasks:" -A 2
   Tasks:    3 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)

  $ HASH=$(cat tmp.log | grep -E "omit-keys:omit-keys-task-with-deps.* executing .*" | awk '{print $5}')
  $ tar -tf $TARGET_DIR/.turbo/cache/$HASH.tar.zst;
  apps/omit-keys/.turbo/turbo-omit-keys-task-with-deps.log
  apps/omit-keys/out/
  apps/omit-keys/out/.keep
  apps/omit-keys/out/foo.min.txt
