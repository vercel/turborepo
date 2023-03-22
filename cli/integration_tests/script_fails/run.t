Setup
  $ . ${TESTDIR}/../setup.sh
  $ . ${TESTDIR}/setup.sh $(pwd)

# Running logs error
  $ ${TURBO} build
  \xe2\x80\xa2 Packages in scope: my-app, util (esc)
  \xe2\x80\xa2 Running build in 2 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  util:build: cache miss, executing 64b207b416059287
  util:build: 
  util:build: > build
  util:build: > echo error && exit 1
  util:build: 
  util:build: error
  util:build: npm ERR! Lifecycle script `build` failed with error: 
  util:build: npm ERR! Error: command failed 
  util:build: npm ERR!   in workspace: util 
  util:build: npm ERR!   at location: .* (re)
  util:build: ERROR: command finished with error: command \(.*\) npm run build exited \(1\) (re)
  command \(.*\) npm run build exited \(1\) (re)
  
   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
   ERROR  run failed: command  exited (1)
  [1]
