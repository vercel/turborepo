Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Running with --filter works and exits
  $ ${TURBO} run build --filter=main
  \xe2\x80\xa2 Packages in scope:  (esc)
  \xe2\x80\xa2 Running build in 0 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
  No tasks were executed as part of this run.
  
   Tasks:    0 successful, 0 total
  Cached:    0 cached, 0 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Running non-existent task with --filter still throws error
  $ ${TURBO} run doesnotexist --filter=main
  ERROR  run failed: error preparing engine: Could not find the following tasks in project: doesnotexist
  Turbo error: error preparing engine: Could not find the following tasks in project: doesnotexist
  [1]
  