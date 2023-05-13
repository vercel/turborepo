  $ . ${TESTDIR}/../setup.sh non-monorepo npm
  8\.\d+\.\d (re)
# run twice and make sure it works
  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false npx turbo build lint --output-logs=none
  \xe2\x80\xa2 Running build, lint (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9ms]+  (re)
  
  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false npx turbo build lint --output-logs=none
  \xe2\x80\xa2 Running build, lint (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  $ git diff
