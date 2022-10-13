Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check error is properly reported
Note that npm reports any failed script as exit code 1, even though we "exit 2"
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:error: cache miss, executing e37081a1eb5d045f
  my-app:error: 
  my-app:error: > error
  my-app:error: > echo 'intentionally failing' && exit 2
  my-app:error: 
  my-app:error: intentionally failing
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*/run.t/apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*/run\.t/apps/my-app\) npm run error exited \(1\) (re)
  command \(.*/run.t/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Make sure it isn't cached
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:error: cache miss, executing e37081a1eb5d045f
  my-app:error: 
  my-app:error: > error
  my-app:error: > echo 'intentionally failing' && exit 2
  my-app:error: 
  my-app:error: intentionally failing
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*/run.t/apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*/run\.t/apps/my-app\) npm run error exited \(1\) (re)
  command \(.*/run.t/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
