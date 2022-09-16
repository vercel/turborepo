Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run build --single-package
  \xe2\x80\xa2 Running build (esc)
  build: cache miss, executing 3ba5bda94a58b0bb
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+ms  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run build --single-package
  \xe2\x80\xa2 Running build (esc)
  build: cache hit, replaying output 3ba5bda94a58b0bb
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+ms >>> FULL TURBO (re)
  
