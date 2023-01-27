Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

Check my-app#build output
  $ ${TURBO} run build
  \xe2\x80\xa2 Packages in scope: //, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing 04c404a8edf3d3cb
  util:build: 
  util:build: > build
  util:build: > echo 'building'
  util:build: 
  util:build: building
  my-app:build: cache miss, executing 4f4f453dc15cbe8c
  my-app:build: 
  my-app:build: > build
  my-app:build: > echo 'building'
  my-app:build: 
  my-app:build: building
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
