# Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh ordered

# Build in grouped order.
  $ ${TURBO} run build --log-order grouped --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing [0-9a-f]+ (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building && sleep 1 && echo done
  my-app:build: 
  my-app:build: building
  my-app:build: done
  util:build: cache bypass, force executing [0-9a-f]+ (re)
  util:build: 
  util:build: > build
  util:build: > sleep 0.5 && echo building && sleep 1 && echo completed
  util:build: 
  util:build: building
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  

# We can get the same behavior with an env var
  $ TURBO_LOG_ORDER=grouped ${TURBO} run build --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing [0-9a-f]+ (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building && sleep 1 && echo done
  my-app:build: 
  my-app:build: building
  my-app:build: done
  util:build: cache bypass, force executing [0-9a-f]+ (re)
  util:build: 
  util:build: > build
  util:build: > sleep 0.5 && echo building && sleep 1 && echo completed
  util:build: 
  util:build: building
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
# The flag wins over the env var
  $ TURBO_LOG_ORDER=stream ${TURBO} run build --log-order grouped --force
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache bypass, force executing [0-9a-f]+ (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building && sleep 1 && echo done
  my-app:build: 
  my-app:build: building
  my-app:build: done
  util:build: cache bypass, force executing [0-9a-f]+ (re)
  util:build: 
  util:build: > build
  util:build: > sleep 0.5 && echo building && sleep 1 && echo completed
  util:build: 
  util:build: building
  util:build: completed
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  