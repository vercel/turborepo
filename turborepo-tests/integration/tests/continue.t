Setup
  $ . ${TESTDIR}/../../helpers/setup.sh
  $ . ${TESTDIR}/_helpers/setup_monorepo.sh $(pwd) monorepo_dependency_error
Run without --continue
  $ ${TURBO} build
  \xe2\x80\xa2 Packages in scope: my-app, other-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing 3494007308f52ad6
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)/apps/some-lib  (re)
  some-lib:build: ERROR: command finished with error: command \((.*)/apps/some-lib\) npm run build exited \(1\) (re)
  command \((.*)/apps/some-lib\) npm run build exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Run without --continue, and with only errors.
  $ ${TURBO} build --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: my-app, other-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing 3494007308f52ad6
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)/apps/some-lib  (re)
  some-lib:build: ERROR: command finished with error: command \((.*)/apps/some-lib\) npm run build exited \(1\) (re)
  command \((.*)/apps/some-lib\) npm run build exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]

Run with --continue
  $ ${TURBO} build --output-logs=errors-only --continue
  \xe2\x80\xa2 Packages in scope: my-app, other-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing 3494007308f52ad6
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)/apps/some-lib  (re)
  some-lib:build: command finished with error, but continuing...
  other-app:build: cache miss, executing af6505fe5634a5f5
  other-app:build: 
  other-app:build: > build
  other-app:build: > exit 3
  other-app:build: 
  other-app:build: npm ERR! Lifecycle script `build` failed with error: 
  other-app:build: npm ERR! Error: command failed 
  other-app:build: npm ERR!   in workspace: other-app 
  other-app:build: npm ERR!   at location: (.*)/apps/other-app  (re)
  other-app:build: command finished with error, but continuing...
  command \((.*)/apps/some-lib\) npm run build exited \(1\) (re)
  command \((.*)/apps/other-app\) npm run build exited \(1\) (re)
  
   Tasks:    1 successful, 3 total
  Cached:    0 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
