Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check error is properly reported
Note that npm reports any failed script as exit code 1, even though we "exit 2"
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache miss, executing 5eaa36f5c1d36f8d
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo 'working'
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 19a93de426a177b1
  my-app:error: 
  my-app:error: > error
  my-app:error: > echo 'intentionally failing' && exit 2
  my-app:error: 
  my-app:error: intentionally failing
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*/run\.t(-\d+)?/apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Make sure error isn't cached
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache hit, replaying output 5eaa36f5c1d36f8d
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo 'working'
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 19a93de426a177b1
  my-app:error: 
  my-app:error: > error
  my-app:error: > echo 'intentionally failing' && exit 2
  my-app:error: 
  my-app:error: intentionally failing
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*/run\.t(-\d+)?/apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Running with --output-mode=errors-only gives error output only
  $ ${TURBO} --output-logs=errors-only error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:error: ERROR: command finished with error: command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  my-app:error: 
  my-app:error: > error
  my-app:error: > echo 'intentionally failing' && exit 2
  my-app:error: 
  my-app:error: intentionally failing
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*/run\.t(-\d+)?/apps/my-app  (re)
  command \(.*/run\.t(-\d+)?/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
