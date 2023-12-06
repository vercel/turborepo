Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_with_root_dep pnpm@7.25.1

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune docs
  Generating pruned monorepo for docs in .*(\/|\\)out (re)
   - Added docs
   - Added shared
   - Added util
  $ cd out && ${TURBO} run new-task
  \xe2\x80\xa2 Packages in scope: docs, shared, util (esc)
  \xe2\x80\xa2 Running new-task in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  docs:new-task: cache miss, executing caf7e46550cd3151
  docs:new-task: 
  docs:new-task: > docs@ new-task .*out(\/|\\)apps(\/|\\)docs (re)
  docs:new-task: > echo building
  docs:new-task: 
  docs:new-task: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


