Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run test
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  build: cache miss, executing fbef1dba65f21ba4
=======
  build: cache miss, executing b69493e87ea97b0e
>>>>>>> 37c3c596f1 (chore: update integration tests)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
<<<<<<< HEAD
  test: cache miss, executing 75187c3aff97a0a8
=======
  test: cache miss, executing 29571ce1244345cc
>>>>>>> 37c3c596f1 (chore: update integration tests)
  test: 
  test: > test
  test: > cat foo.txt
  test: 
  test: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
<<<<<<< HEAD
=======
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
<<<<<<< HEAD
=======
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
Run a second time, verify caching works because there is a config
  $ ${TURBO} run test
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  build: cache hit, replaying logs fbef1dba65f21ba4
=======
  build: cache hit, replaying logs b69493e87ea97b0e
>>>>>>> 37c3c596f1 (chore: update integration tests)
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
<<<<<<< HEAD
  test: cache hit, replaying logs 75187c3aff97a0a8
=======
  test: cache hit, replaying logs 29571ce1244345cc
>>>>>>> 37c3c596f1 (chore: update integration tests)
  test: 
  test: > test
  test: > cat foo.txt
  test: 
  test: building
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
<<<<<<< HEAD
=======
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
<<<<<<< HEAD
=======
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
Run with --output-logs=hash-only
  $ ${TURBO} run test --output-logs=hash-only
  \xe2\x80\xa2 Running test (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
<<<<<<< HEAD
  build: cache hit, suppressing logs fbef1dba65f21ba4
  test: cache hit, suppressing logs 75187c3aff97a0a8
=======
  build: cache hit, suppressing logs b69493e87ea97b0e
  test: cache hit, suppressing logs 29571ce1244345cc
>>>>>>> 37c3c596f1 (chore: update integration tests)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
<<<<<<< HEAD
=======
>>>>>>> b668d5abb3 (chore: remove task dotEnv field)
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
  
