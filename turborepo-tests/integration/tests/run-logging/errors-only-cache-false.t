Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh run_logging

Test for issue #6677: --output-logs errors-only should work with cache:false tasks

# Successful task with cache:false and outputLogs: errors-only
# Expected: No output should be shown (output suppressed on success)
  $ ${TURBO} run nocache-success
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running nocache-success in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


# Successful task with cache:false and --output-logs=errors-only flag
# Expected: No output should be shown (output suppressed on success)
  $ ${TURBO} run build --output-logs=errors-only --force
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  


# Failed task with cache:false and outputLogs: errors-only
# Expected: Output SHOULD be shown because task failed
  $ ${TURBO} run nocache-error
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running nocache-error in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:nocache-error: cache miss, executing [a-z0-9]{16} (re)
  app-a:nocache-error: 
  app-a:nocache-error: > nocache-error
  app-a:nocache-error: > echo nocache-error-app-a && exit 1
  app-a:nocache-error: 
  app-a:nocache-error: nocache-error-app-a
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: npm error.* (re)
  app-a:nocache-error: ERROR: command finished with error: command .* exited \(1\) (re)
  app-a#nocache-error: command .* exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    app-a#nocache-error
  
   ERROR  run failed: command  exited (1)
  [1]
