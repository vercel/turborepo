Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh root_deps

Warm the cache
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache miss, executing 2e4e289aeb0b6055
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)

Confirm cache hit
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache hit, suppressing logs 2e4e289aeb0b6055

   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)

Change a root internal dependency
  $ touch packages/util/important.txt
All tasks should be a cache miss, even ones that don't depend on changed package 
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache miss, executing ebc7de7a6c0c5543
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  

Change a file that is git ignored
  $ mkdir packages/util/dist
  $ touch packages/util/dist/unused.txt
Cache hit since only tracked files contribute to root dep hash
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache hit, suppressing logs ebc7de7a6c0c5543
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
