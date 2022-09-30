Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --single-package
  \xe2\x80\xa2 Running build (esc)
   INFO  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache miss, executing e491d0044f4b9b90
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run build --single-package
  \xe2\x80\xa2 Running build (esc)
   INFO  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, replaying output e491d0044f4b9b90
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  