Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run test
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache miss, executing 7ece7b62aad25615
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  test: cache miss, executing cb5839f7284aa5f3
  test: 
  test: > test
  test: > cat foo.txt
  test: 
  test: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run test
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, replaying logs 7ece7b62aad25615
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  test: cache hit, replaying logs cb5839f7284aa5f3
  test: 
  test: > test
  test: > cat foo.txt
  test: 
  test: building
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=hash-only
  $ ${TURBO} run test --output-logs=hash-only
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, suppressing logs 7ece7b62aad25615
  test: cache hit, suppressing logs cb5839f7284aa5f3
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=errors-only
  $ ${TURBO} run test --output-logs=errors-only
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=none
  $ ${TURBO} run test --output-logs=none
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
