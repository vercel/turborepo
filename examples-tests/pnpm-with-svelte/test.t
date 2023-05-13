  $ . ${TESTDIR}/../setup.sh with-svelte pnpm
  6.26.1
# run twice and make sure it works
  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false pnpm run build lint -- --output-logs=none
  
  \> @ build (.*)/test.t (re)
  \> turbo run build "lint" "--output-logs=none" (re)
  
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    0 cached, 5 total
    Time:\s*[\.0-9ms]+  (re)
  
  $ TURBO_TEAM="" TURBO_REMOTE_ONLY=false pnpm run build lint -- --output-logs=none
  
  \> @ build (.*)/test.t (re)
  \> turbo run build "lint" "--output-logs=none" (re)
  
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 4 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    5 cached, 5 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  $ git diff
