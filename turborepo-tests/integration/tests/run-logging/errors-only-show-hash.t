Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh run_logging_errors_only_show_hash

Test that errorsOnlyShowHash future flag shows hash on successful cache miss
  $ ${TURBO} run build --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:build: cache miss, executing [0-9a-f]+ \(only logging errors\) (re)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Test that errorsOnlyShowHash shows hash on cache hit
  $ ${TURBO} run build --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:build: cache hit, replaying logs \(no errors\) [0-9a-f]+ (re)
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s.* (re)
  

Test that errorsOnlyShowHash with outputLogs: errors-only in turbo.json shows hash on success
  $ ${TURBO} run buildsuccess
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running buildsuccess in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:buildsuccess: cache miss, executing [0-9a-f]+ \(only logging errors\) (re)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Test that errorsOnlyShowHash shows hash on cache hit for turbo.json configured task
  $ ${TURBO} run buildsuccess
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running buildsuccess in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:buildsuccess: cache hit, replaying logs \(no errors\) [0-9a-f]+ (re)
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s.* (re)
  

Test that errorsOnlyShowHash still shows full logs on error (but does NOT repeat cache miss)
  $ ${TURBO} run builderror
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running builderror in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:builderror: cache miss, executing [0-9a-f]+ \(only logging errors\) (re)
  app-a:builderror: 
  app-a:builderror: > builderror
  app-a:builderror: > echo error-builderror-app-a && exit 1
  app-a:builderror: 
  app-a:builderror: error-builderror-app-a
  app-a:builderror: npm ERR! Lifecycle script `builderror` failed with error: 
  app-a:builderror: npm ERR! Error: command failed 
  app-a:builderror: npm ERR!   in workspace: app-a 
  app-a:builderror: npm ERR!   at location: .* (re)
  app-a:builderror: ERROR: command finished with error: command .*npm(?:\.cmd)? run builderror exited \(1\) (re)
  app-a#builderror: command .*npm(?:\.cmd)? run builderror exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    app-a#builderror
  
   ERROR  run failed: command  exited (1)
  [1]

