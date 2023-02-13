Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# The override-values-task-with-deps configures dependsOn in the root turbo.json.
# The workspace does not have a turbo.json config. This test checks that both regular dependencies
# and Topological dependencies are retained from the root config.

# 1. First run, assert that dependet tasks run `dependsOn`
  $ ${TURBO} run override-values-task-with-deps --filter=override-values > tmp.log
# Validate in pieces. `omit-key` task has two dependsOn values, and those tasks
# can run in non-deterministic order. So we need to validate the logs in the pieces.
  $ cat tmp.log | grep "in scope" -A 2
  \xe2\x80\xa2 Packages in scope: override-values (esc)
  \xe2\x80\xa2 Running override-values-task-with-deps in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)

  $ cat tmp.log | grep "override-values:override-values-task-with-deps"
  override-values:override-values-task-with-deps: cache miss, executing cf35abb7b46ffad7
  override-values:override-values-task-with-deps: 
  override-values:override-values-task-with-deps: > override-values-task-with-deps
  override-values:override-values-task-with-deps: > echo "running override-values-task-with-deps" > out/foo.min.txt
  override-values:override-values-task-with-deps: 

  $ cat tmp.log | grep "override-values:override-values-underlying-task"
  override-values:override-values-underlying-task: cache miss, executing 783a94e433071496
  override-values:override-values-underlying-task: 
  override-values:override-values-underlying-task: > override-values-underlying-task
  override-values:override-values-underlying-task: > echo "running override-values-underlying-task"
  override-values:override-values-underlying-task: 
  override-values:override-values-underlying-task: running override-values-underlying-task

  $ cat tmp.log | grep "blank-pkg:override-values-underlying-topo-task"
  blank-pkg:override-values-underlying-topo-task: cache miss, executing 0e2630802fda80c3
  blank-pkg:override-values-underlying-topo-task: 
  blank-pkg:override-values-underlying-topo-task: > override-values-underlying-topo-task
  blank-pkg:override-values-underlying-topo-task: > echo "override-values-underlying-topo-task from blank-pkg"
  blank-pkg:override-values-underlying-topo-task: 
  blank-pkg:override-values-underlying-topo-task: override-values-underlying-topo-task from blank-pkg

  $ cat tmp.log | grep "Tasks:" -A 2
   Tasks:    3 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
