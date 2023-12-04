https://github.com/vercel/turbo/issues/6677
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh $(pwd) run_logging
  $ . ${TESTDIR}/../../../helpers/replace_turbo_config.sh $(pwd) "cache-false.json"

  $ turbo run build --output-logs errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  

