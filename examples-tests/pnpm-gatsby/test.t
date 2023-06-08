  $ . ${TESTDIR}/setup.sh with-gatsby pnpm
  6.26.1
# run twice and make sure it works
  $ pnpm run build lint -- --output-logs=none
  
  \> with-gatsby@0.0.0 build (.*)/pnpm-gatsby.t (re)
  \> turbo build "lint" "--output-logs=none" (re)
  
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, tsconfig, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    0 cached, 5 total
    Time:\s*[\.0-9ms]+  (re)
  

  $ pnpm run build lint -- --output-logs=none
  
  \> with-gatsby@0.0.0 build (.*)/pnpm-gatsby.t (re)
  \> turbo build "lint" "--output-logs=none" (re)
  
  \xe2\x80\xa2 Packages in scope: docs, eslint-config-custom, tsconfig, ui, web (esc)
  \xe2\x80\xa2 Running build, lint in 5 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    5 successful, 5 total
  Cached:    3 cached, 5 total
    Time:\s*[\.0-9ms]+  (re)
  
