Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check
  $ ${TURBO} run test --single-package
  \xe2\x80\xa2 Running test (esc)
   INFO  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache miss, executing 6f7bc2246be20691
  build:
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache miss, executing 0636940bc33feefa
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
Run a second time, verify caching works because there is a config
  $ ${TURBO} run test --single-package
  \xe2\x80\xa2 Running test (esc)
   INFO  \xe2\x80\xa2 Remote caching disabled (esc)
  build: cache hit, replaying output 6f7bc2246be20691
  build:
  build: > build
  build: > echo 'building' > foo
  build: 
  test: cache hit, replaying output 0636940bc33feefa
  test: 
  test: > test
  test: > [[ ( -f foo ) && $(cat foo) == 'building' ]]
  test: 
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  