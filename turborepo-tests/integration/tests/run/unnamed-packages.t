Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh nested_packages

Run a build. In this test, the fixture is set up so that the nested package
which does not have a name should be ignored. We should process it but filter.
  $ ${TURBO} build
  \xe2\x80\xa2 Packages in scope: my-app (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  my-app:build: cache miss, executing [0-9a-f]+ (re)
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo building
  my-app:build: 
  my-app:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:    (.+)  (re)
  
