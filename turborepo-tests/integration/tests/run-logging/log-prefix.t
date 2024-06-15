Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh run_logging

# Run for the first time with --log-prefix=none
  $ ${TURBO} run build --log-prefix=none
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  cache miss, executing 612027951a2848ce
  
  \> build (re)
  \> echo build-app-a (re)
  
  build-app-a
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
# Check that the cached logs don't have prefixes
  $ cat app-a/.turbo/turbo-build.log
  
  \> build (re)
  \> echo build-app-a (re)
  
  build-app-a

# Running again should get a cache hit and no prefixes
  $ ${TURBO} run build --log-prefix=none
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  cache hit, replaying logs 612027951a2848ce
  
  \> build (re)
  \> echo build-app-a (re)
  
  build-app-a
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
# Running again withuot `--log-prefix` should get a cache hit, but should print prefixes this time
  $ ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:build: cache hit, replaying logs 612027951a2848ce
  app-a:build: 
  app-a:build: > build
  app-a:build: > echo build-app-a
  app-a:build: 
  app-a:build: build-app-a
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

# Running with bogus option
  $ ${TURBO} run build --log-prefix=blah
   ERROR  invalid value 'blah' for '--log-prefix <LOG_PREFIX>'
    [possible values: auto, none, task]
  
  For more information, try '--help'.
  
  [1]

# Running with missing value for option
  $ ${TURBO} run build --log-prefix
   ERROR  a value is required for '--log-prefix <LOG_PREFIX>' but none was supplied
    [possible values: auto, none, task]
  
  For more information, try '--help'.
  
  [1]
