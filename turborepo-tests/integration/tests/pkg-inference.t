Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

# Run as if called by global turbo
  $ TURBO_INVOCATION_DIR=$(pwd)/packages/util ${TURBO} build --skip-infer
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing 1ce33e04f265f95c
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
