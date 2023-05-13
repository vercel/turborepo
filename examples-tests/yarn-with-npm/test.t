  $ . ${TESTDIR}/../setup.sh with-npm yarn
  1.22.17

# run twice and make sure it works
  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false yarn turbo build lint --output-logs=none
  yarn run v1.22.17
  \$ (.*)node_modules/.bin/turbo build lint --output-logs=none (re)
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, tsconfig, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    0 cached, 5 total
    Time:\s*[\.0-9ms]+  (re)
  
  Done in [\.0-9]+m?s\. (re)

  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false yarn turbo build lint --output-logs=none
  yarn run v1.22.17
  \$ (.*)node_modules/.bin/turbo build lint --output-logs=none (re)
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, tsconfig, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    5 cached, 5 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  Done in [\.0-9]+m?s\. (re)

  $ git diff
