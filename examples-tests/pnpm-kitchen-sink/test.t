  $ . ${TESTDIR}/../setup.sh kitchen-sink pnpm
  6.26.1
# run twice and make sure it works
  $ pnpm run build lint -- --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: admin, api, blog, eslint-config-custom, eslint-config-custom-server, jest-presets, logger, storefront, tsconfig, ui (esc)
  \xe2\x80\xa2 Running build, lint in 10 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    11 successful, 11 total
  Cached:    0 cached, 11 total
    Time:\s*[\.0-9ms]+  (re)
  
  $ pnpm run build lint -- --output-logs=errors-only
  
  \> @ build (.*)/test.t (re)
  \> turbo build "lint" "--output-logs=errors-only" (re)
  
  \xe2\x80\xa2 Packages in scope: admin, api, blog, eslint-config-custom, eslint-config-custom-server, jest-presets, logger, storefront, tsconfig, ui (esc)
  \xe2\x80\xa2 Running build, lint in 10 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    11 successful, 11 total
  Cached:    11 cached, 11 total
    Time:\s*[\.0-9ms]+ >>> FULL TURBO (re)
  
  $ git diff
