# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh ordered

# Build as if we are in Github Actions
Note that we need to use (re) for lines that start with '> '
because otherwise prysk interprets them as multiline commands
  $ GITHUB_ACTIONS=1 ${TURBO} run build --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::my-app:build
  cache bypass, force executing c1d33a8183d8cf0b
  
  >\sbuild (re)
  \>\secho building && sleep 1 && echo done (re)
  
  building
  done
  ::endgroup::
  ::group::util:build
  cache bypass, force executing ff1050c513839636
  
  >\sbuild (re)
  \>\ssleep 0.5 && echo building && sleep 1 && echo completed (re)
  
  building
  completed
  ::endgroup::
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Build as if we are in Github Actions with a task log prefix.
  $ GITHUB_ACTIONS=1 ${TURBO} run build --force --log-prefix="task" --filter=util
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::util:build
  util:build: cache bypass, force executing ff1050c513839636
  util:build: 
  util:build: > build
  util:build: > sleep 0.5 && echo building && sleep 1 && echo completed
  util:build: 
  util:build: building
  util:build: completed
  ::endgroup::
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Verify that errors are grouped properly
  $ GITHUB_ACTIONS=1 ${TURBO} run fail
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running fail in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  ::group::util:fail
  cache miss, executing 122cca10fdcda4f0
  
  \> fail (re)
  \> echo failing; exit 1 (re)
  
  failing
  npm ERR! Lifecycle script `fail` failed with error: 
  npm ERR! Error: command failed 
  npm ERR!   in workspace: util 
  npm ERR\!   at location: (.*)(\/|\\)packages(\/|\\)util  (re)
  \[ERROR\] command finished with error: command \((.*)(\/|\\)packages(\/|\\)util\) (.*)npm(?:\.cmd)? run fail exited \(1\) (re)
  ::endgroup::
  ::error::util#fail: command \(.*(\/|\\)packages(\/|\\)util\) (.*)npm(?:\.cmd)? run fail exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    util#fail
  
   ERROR  run failed: command  exited (1)
  [1]



