# Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd) ordered

# Build as if we are in Github Actions
Note that we need to use (re) for lines that start with '> '
because otherwise prysk interprets them as multiline commands
  $ export GITHUB_ACTIONS=1
  $ ${TURBO} run build --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::my-app:build
  cache bypass, force executing 4c3a4e8d472d74b2
  
  >\sbuild (re)
  >\secho 'building' && sleep 1 && echo 'done' (re)
  
  building
  done
  ::endgroup::
  ::group::util:build
  cache bypass, force executing 90d7154e362e3386
  
  >\sbuild (re)
  >\ssleep 0.5 && echo 'building' && sleep 1 && echo 'completed' (re)
  
  building
  completed
  ::endgroup::
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
Verify that errors are grouped properly
  $ ${TURBO} run fail
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running fail in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::util:fail
  cache miss, executing 9bf2c727d81cf834
  
  \> fail (re)
  \> echo 'failing'; exit 1 (re)
  
  failing
  npm ERR! Lifecycle script `fail` failed with error: 
  npm ERR! Error: command failed 
  npm ERR!   in workspace: util 
  npm ERR\!   at location: (.*)/packages/util  (re)
  \[ERROR\] command finished with error: command \((.*)/packages/util\) npm run fail exited \(1\) (re)
  ::endgroup::
  ::error::util#fail: command \(.*/packages/util\) npm run fail exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    util#fail
  
   ERROR  run failed: command  exited (1)
  [1]



