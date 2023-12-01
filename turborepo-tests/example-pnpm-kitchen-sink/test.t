  $ . ${TESTDIR}/../helpers/examples_setup.sh kitchen-sink pnpm

# run twice and make sure it works
  $ pnpm run build lint --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: admin, api, blog, eslint-config-custom, jest-presets, logger, storefront, tsconfig, ui (esc)
  \xe2\x80\xa2 Running build, lint in 9 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    12 successful, 12 total
  Cached:    0 cached, 12 total
    Time:\s*[\.0-9ms]+  (re)
  

  $ pnpm run build lint --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: admin, api, blog, eslint-config-custom, jest-presets, logger, storefront, tsconfig, ui (esc)
  \xe2\x80\xa2 Running build, lint in 9 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    12 successful, 12 total
  Cached:    12 cached, 12 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  

  $ git diff
