Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh global_deps

Run a build
  $ ${TURBO} build -F my-app --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 81165ceb4ed0e31f
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

  $ echo "new text" > global_deps/foo.txt
  $ ${TURBO} build -F my-app --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing 7bc88ba3e84628fd
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
