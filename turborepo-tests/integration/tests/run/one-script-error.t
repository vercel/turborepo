Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_one_script_error

Check error is properly reported
Note that npm reports any failed script as exit code 1, even though we "exit 2"
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache miss, executing ffe52e38e68be53e
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo working
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 8d3be716599fe376
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps(\/|\\)my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*apps(\/|\\)my-app\) (.*)npm(?:\.cmd)? run error exited \(1\) (re)
  my-app#error: command \(.*apps(\/|\\)my-app\) (.*)npm(?:\.cmd)? run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    my-app#error
  
   ERROR  run failed: command  exited (1)
  [1]

Make sure error isn't cached
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache hit, replaying logs ffe52e38e68be53e
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo working
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 8d3be716599fe376
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps(\/|\\)my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*apps(\/|\\)my-app\) (.*)npm(?:\.cmd)? run error exited \(1\) (re)
  my-app#error: command \(.*apps(\/|\\)my-app\) (.*)npm(?:\.cmd)? run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    my-app#error
  
   ERROR  run failed: command  exited (1)
  [1]

Make sure error code isn't swallowed with continue
  $ ${TURBO} okay2 --continue
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running okay2 in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache hit, replaying logs ffe52e38e68be53e
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo working
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 8d3be716599fe376
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps(\/|\\)my-app  (re)
  my-app:error: command finished with error, but continuing...
  my-app:okay2: cache miss, executing 13c728e793c08f30
  my-app:okay2: 
  my-app:okay2: > okay2
  my-app:okay2: > echo working
  my-app:okay2: 
  my-app:okay2: working
  my-app#error: command \((.*)(\/|\\)apps(\/|\\)my-app\) (.*)npm(?:\.cmd)? run error exited \(1\) (re)
  
   Tasks:    2 successful, 3 total
  Cached:    1 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    my-app#error
  
   ERROR  run failed: command  exited (1)
  [1]
