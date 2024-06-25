Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh root_deps

Verify that no packages are in scope
  $ ${TURBO} build --filter='[HEAD]' --dry=json | jq '.packages'
  []

Warm the cache
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache miss, executing 6a4c300cb14847b0
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  

Confirm cache hit
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache hit, suppressing logs 6a4c300cb14847b0
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s+[.0-9]+m?s >>> FULL TURBO (re)
  


Change a root internal dependency
  $ touch packages/util/important.txt
All tasks should be a cache miss, even ones that don't depend on changed package 
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache miss, executing 34787620f332fb95
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s+[.0-9]+m?s  (re)
  

Verify that all packages are in scope on a internal root dep change
  $ ${TURBO} build --filter='[HEAD]' --dry=json | jq '.packages'
  [
    "//",
    "another",
    "my-app",
    "util",
    "yet-another"
  ]

Change a file that is git ignored
  $ mkdir packages/util/dist
  $ touch packages/util/dist/unused.txt
Cache hit since only tracked files contribute to root dep hash
  $ ${TURBO} build --filter=another --output-logs=hash-only
  \xe2\x80\xa2 Packages in scope: another (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  another:build: cache hit, suppressing logs 34787620f332fb95
  
   Tasks:    1 successful, 1 total
  Cached:    1 cached, 1 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  

Verify that all packages are considered dependants of a root dep
  $ ${TURBO} build --filter='...util' --dry=json | jq '.packages'
  [
    "//",
    "another",
    "my-app",
    "util",
    "yet-another"
  ]
Verify that a root dependency is considered a dependency of all packages
  $ ${TURBO} build --filter='another...' --dry=json | jq '.packages'
  [
    "another",
    "util",
    "yet-another"
  ]
