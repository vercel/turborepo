# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh ordered

# Build in stream order. All the .*'s are unpredictable lines, however the amount of lines is predictable.
  $ ${TURBO} run build --log-order stream --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  util:build: building
  my-app:build: done
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
The env var set to stream works (this is default, so this test doesn't guarantee the env var is "working"),
it just guarantees setting this env var won't crash.
  $ TURBO_LOG_ORDER=stream ${TURBO} run build --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  util:build: building
  my-app:build: done
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
The flag wins over the env var
  $ TURBO_LOG_ORDER=grouped ${TURBO} run build --log-order stream --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  (my-app|util):build: cache bypass, force executing [0-9a-f]+ (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  .* (re)
  util:build: building
  my-app:build: done
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  