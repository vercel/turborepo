Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Verbosity level 1
  $ ${TURBO} build -v --filter=util --force
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 1ce33e04f265f95c
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  $ ${TURBO} build --verbosity=1 --filter=util --force
  \xe2\x80\xa2 Packages in scope: util (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache bypass, force executing 1ce33e04f265f95c
  util:build: 
  util:build: > build
  util:build: > echo building
  util:build: 
  util:build: building
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

Verbosity level 2
  $ ${TURBO} build -vv --filter=util --force 1> VERBOSEVV 2>&1
  $ grep --quiet "[DEBUG]" VERBOSEVV

  $ ${TURBO} build --verbosity=2 --filter=util --force 1> VERBOSE2 2>&1
  $ grep --quiet "[DEBUG]" VERBOSE2

Make sure users can only use one verbosity flag
  $ ${TURBO} build -v --verbosity=1
   ERROR  the argument '-v...' cannot be used with '--verbosity <COUNT>'
  
  Usage: turbo(\.exe)? \[OPTIONS\] \[COMMAND\] (re)
  
  For more information, try '--help'.
  
  [1]
