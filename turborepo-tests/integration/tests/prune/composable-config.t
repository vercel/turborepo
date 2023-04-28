Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) monorepo_with_root_dep

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=docs
  Generating pruned monorepo for docs in .*\/out (re)
   - Added docs
   - Added shared
   - Added util
  $ cd out && ${TURBO} run new-task
  \xe2\x80\xa2 Packages in scope: docs, shared, util (esc)
  \xe2\x80\xa2 Running new-task in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  docs:new-task: cache miss, executing 9c08c9fca02fee6e
  docs:new-task: 
  docs:new-task: > docs@ new-task .*out/apps/docs (re)
  docs:new-task: > echo 'running new task'
  docs:new-task: 
  docs:new-task: running new task
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


