Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh monorepo_dependency_error
Run without --continue
  $ ${TURBO} build --filter my-app...
  \xe2\x80\xa2 Packages in scope: base-lib, my-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  base-lib:build: cache miss, executing 664e9be50059c3a2
  base-lib:build: 
  base-lib:build: > build
  base-lib:build: > echo 'working'
  base-lib:build: 
  base-lib:build: working
  some-lib:build: cache miss, executing ba873583be62d113
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)(\/|\\)apps(\/|\\)some-lib  (re)
  some-lib:build: ERROR: command finished with error: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  some-lib#build: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    some-lib#build
  
   ERROR  run failed: command  exited (1)
  [1]


Run without --continue, and with only errors.
  $ ${TURBO} build --output-logs=errors-only --filter my-app...
  \xe2\x80\xa2 Packages in scope: base-lib, my-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing ba873583be62d113
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)(\/|\\)apps(\/|\\)some-lib  (re)
  some-lib:build: ERROR: command finished with error: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  some-lib#build: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  
   Tasks:    1 successful, 2 total
  Cached:    1 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    some-lib#build
  
   ERROR  run failed: command  exited (1)
  [1]

Run with --continue
  $ ${TURBO} build --output-logs=errors-only --filter my-app... --continue
  \xe2\x80\xa2 Packages in scope: base-lib, my-app, some-lib (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing ba873583be62d113
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)(\/|\\)apps(\/|\\)some-lib  (re)
  some-lib:build: command finished with error, but continuing...
  some-lib#build: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  
   Tasks:    2 successful, 3 total
  Cached:    1 cached, 3 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    some-lib#build
  
   ERROR  run failed: command  exited (1)
  [1]

Run with --continue=dependencies-successful
  $ ${TURBO} build --output-logs=errors-only --continue=dependencies-successful
  \xe2\x80\xa2 Packages in scope: base-lib, my-app, other-app, some-lib, yet-another-lib (esc)
  \xe2\x80\xa2 Running build in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  some-lib:build: cache miss, executing ba873583be62d113
  some-lib:build: 
  some-lib:build: > build
  some-lib:build: > exit 2
  some-lib:build: 
  some-lib:build: npm ERR! Lifecycle script `build` failed with error: 
  some-lib:build: npm ERR! Error: command failed 
  some-lib:build: npm ERR!   in workspace: some-lib 
  some-lib:build: npm ERR!   at location: (.*)(\/|\\)apps(\/|\\)some-lib  (re)
  some-lib:build: command finished with error, but continuing...
  other-app:build: cache miss, executing 2f02bb91c86e81a4
  other-app:build: 
  other-app:build: > build
  other-app:build: > exit 3
  other-app:build: 
  other-app:build: npm ERR! Lifecycle script `build` failed with error: 
  other-app:build: npm ERR! Error: command failed 
  other-app:build: npm ERR!   in workspace: other-app 
  other-app:build: npm ERR!   at location: (.*)(\/|\\)apps(\/|\\)other-app  (re)
  other-app:build: command finished with error, but continuing...
  some-lib#build: command \((.*)(\/|\\)apps(\/|\\)some-lib\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  other-app#build: command \((.*)(\/|\\)apps(\/|\\)other-app\) .*npm(?:\.cmd)? run build exited \(1\) (re)
  
   Tasks:    2 successful, 4 total
  Cached:    1 cached, 4 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    other-app#build, some-lib#build
  
   ERROR  run failed: command  exited (1)
  [1]
