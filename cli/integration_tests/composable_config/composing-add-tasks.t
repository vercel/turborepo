Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

  $ ${TURBO} run added-task --skip-infer --filter=add-tasks
  \xe2\x80\xa2 Packages in scope: add-tasks (esc)
  \xe2\x80\xa2 Running added-task in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  
