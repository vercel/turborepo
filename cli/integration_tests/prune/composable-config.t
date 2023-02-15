Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Make sure that the internal util package is part of the prune output
  $ ${TURBO} prune --scope=docs
  Generating pruned monorepo for docs in .*\/out (re)
   - Added docs
   - Added shared
   - Added util
  $ cd out && ${TURBO} run new-task
   WARNING  cannot find a .git folder. Falling back to manual file hashing (which may be slower). If you are running this build in a pruned directory, you can ignore this message. Otherwise, please initialize a git repository in the root of your monorepo
  \xe2\x80\xa2 Packages in scope: docs, shared, util (esc)
  \xe2\x80\xa2 Running new-task in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  docs:new-task: cache miss, executing 89b0cf4ede0c4ae5
  docs:new-task: 
  docs:new-task: > docs@ new-task .*out/apps/docs (re)
  docs:new-task: > echo 'running new task'
  docs:new-task: 
  docs:new-task: running new task
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
