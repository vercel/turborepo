  $ . ${TESTDIR}/../setup.sh non-monorepo npm
  \d+\.\d+\.\d+ (re)
# run twice and make sure it works
  $ npx turbo build lint --output-logs=errors-only
  \xe2\x80\xa2 Running build, lint (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9ms]+  (re)
  
  $ npx turbo build lint --output-logs=errors-only
  \xe2\x80\xa2 Running build, lint (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  $ git diff
