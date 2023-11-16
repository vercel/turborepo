  $ . ${TESTDIR}/../setup.sh basic pnpm
  6.26.1
# run twice and make sure it works
  $ pnpm run build lint -- --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo run build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: @repo/eslint-config, @repo/typescript-config, @repo/ui, docs, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    6 successful, 6 total
  Cached:    0 cached, 6 total
    Time:\s*[\.0-9ms]+  (re)
  
  $ pnpm run build lint -- --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo run build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: @repo/eslint-config, @repo/typescript-config, @repo/ui, docs, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    6 successful, 6 total
  Cached:    6 cached, 6 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  $ git diff
