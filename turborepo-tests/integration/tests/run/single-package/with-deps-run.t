Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run test
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  build: cache miss, executing e48ea8d453fe3216
=======
  build: cache miss, executing 21b3f16d0cd4a89c
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
<<<<<<< HEAD
  test: cache miss, executing 92fa9e0daec0e1ec
=======
  test: cache miss, executing 23c168552034dd2f
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
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
<<<<<<< HEAD
  build: cache hit, replaying logs e48ea8d453fe3216
=======
  build: cache hit, replaying logs 21b3f16d0cd4a89c
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
<<<<<<< HEAD
  test: cache hit, replaying logs 92fa9e0daec0e1ec
=======
  test: cache hit, replaying logs 23c168552034dd2f
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
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
<<<<<<< HEAD
  build: cache hit, suppressing logs e48ea8d453fe3216
  test: cache hit, suppressing logs 92fa9e0daec0e1ec
=======
  build: cache hit, suppressing logs 21b3f16d0cd4a89c
  test: cache hit, suppressing logs 23c168552034dd2f
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
  
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
  
