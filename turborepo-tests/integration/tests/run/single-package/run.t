Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run build
  \xe2\x80\xa2 Running build (esc)
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
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ test -d .turbo/runs/
  [1]

Run a second time, verify caching works because there is a config
  $ ${TURBO} run build
  \xe2\x80\xa2 Running build (esc)
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
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
