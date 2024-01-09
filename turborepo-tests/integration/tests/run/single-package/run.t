Setup
  $ . ${TESTDIR}/../../../../helpers/setup_integration_test.sh single_package

Check
  $ ${TURBO} run build
  \xe2\x80\xa2 Running build (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache miss, executing f09bf783beacf5c9
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
  build: cache hit, replaying logs f09bf783beacf5c9
  build: 
  build: > build
  build: > echo building > foo.txt
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  