Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh composable_config

  $ ${TURBO} run added-task --filter=add-tasks
  \xe2\x80\xa2 Packages in scope: add-tasks (esc)
  \xe2\x80\xa2 Running added-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  add-tasks:added-task: cache miss, executing 0c8333ca7685d45f
  add-tasks:added-task: 
  add-tasks:added-task: > added-task
  add-tasks:added-task: > echo running-added-task > out/foo.min.txt
  add-tasks:added-task: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  
