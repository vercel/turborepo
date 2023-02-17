Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd) ./monorepo

# Run for the first time with --log-prefix=none
  $ ${TURBO} run build --log-prefix=none
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  cache miss, executing 74eb1b46ce8b29d3

  > build (esc)
  > echo 'build app-a' (esc)
  
  build app-a
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)

# Check that the cached logs don't have prefixes
  $ cat app-a/.turbo/turbo-build.log

  > build (esc)
  > echo 'build app-a' (esc)

  build app-a

# Should get a cache hit and no prefixes
  $ ${TURBO} run build --log-prefix=none
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  cache miss, executing 3df2c74b2bfbc724
  
  > build
  > echo 'build app-a'
  
  build app-a

  Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
  Time:\s*[\.0-9]+m?s  (re)

# Should get a cache hit, but should print prefixes this time
  $ ${TURBO} run build # without option
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:build: cache hit, replaying output 3df2c74b2bfbc724
  app-a:build: 
  app-a:build: > build
  app-a:build: > echo 'build app-a'
  app-a:build: 
  app-a:build: build app-a

  Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
  Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
