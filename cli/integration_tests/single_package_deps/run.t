Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --single-package
  \xe2\x80\xa2 Running test (esc)
  build: cache miss, executing fb5ab7cab2c98c77
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache miss, executing 3d586528c591ec52
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+ms  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run test --single-package
  \xe2\x80\xa2 Running test (esc)
  build: cache hit, replaying output fb5ab7cab2c98c77
  build: 
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache hit, replaying output 3d586528c591ec52
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+ms >>> FULL TURBO (re)
  
