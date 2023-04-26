Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd) monorepo_one_script_error

Check error is properly reported
Note that npm reports any failed script as exit code 1, even though we "exit 2"
  $ ${TURBO} error
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache miss, executing 62ff444b3068c13b
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo 'working'
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 7ec8abd964436064
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*apps/my-app\) npm run error exited \(1\) (re)
  command \(.*apps/my-app\) npm run error exited \(1\) (re)
  
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
  my-app:okay: cache hit, replaying output 62ff444b3068c13b
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo 'working'
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 7ec8abd964436064
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps/my-app  (re)
  my-app:error: ERROR: command finished with error: command \(.*apps/my-app\) npm run error exited \(1\) (re)
  command \(.*apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Make sure error code isn't swallowed with continue
  $ ${TURBO} okay2 --continue
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running okay2 in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:okay: cache hit, replaying output 62ff444b3068c13b
  my-app:okay: 
  my-app:okay: > okay
  my-app:okay: > echo 'working'
  my-app:okay: 
  my-app:okay: working
  my-app:error: cache miss, executing 7ec8abd964436064
  my-app:error: 
  my-app:error: > error
  my-app:error: > exit 2
  my-app:error: 
  my-app:error: npm ERR! Lifecycle script `error` failed with error: 
  my-app:error: npm ERR! Error: command failed 
  my-app:error: npm ERR!   in workspace: my-app 
  my-app:error: npm ERR!   at location: .*apps/my-app  (re)
  my-app:error: command finished with error, but continuing...
  my-app:okay2: cache miss, executing 6ec9a564c31e8f12
  my-app:okay2: 
  my-app:okay2: > okay2
  my-app:okay2: > echo 'working'
  my-app:okay2: 
  my-app:okay2: working
  command \((.*)/apps/my-app\) npm run error exited \(1\) (re)
  
   Tasks:    2 successful, 3 total
  Cached:    1 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
