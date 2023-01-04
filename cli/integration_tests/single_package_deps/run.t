Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --single-package
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache miss, executing 8fc80cfff3b64237
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache miss, executing c71366ccd6a86465
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run test --single-package
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, replaying output 8fc80cfff3b64237
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache hit, replaying output c71366ccd6a86465
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=hash-only
  $ ${TURBO} run test --single-package --output-logs=hash-only
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, suppressing output 8fc80cfff3b64237
  test: cache hit, suppressing output c71366ccd6a86465
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=errors-only
  $ ${TURBO} run test --single-package --output-logs=errors-only
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
Run with --output-logs=none
  $ ${TURBO} run test --single-package --output-logs=none
  No local turbo binary found at: .+node_modules/\.bin/turbo (re)
  Running command as global turbo
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
