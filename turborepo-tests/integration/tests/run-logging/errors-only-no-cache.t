Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh run_logging

# Successful non-cached task with --output-logs=errors-only should suppress output
  $ ${TURBO} run nocachebuild --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running nocachebuild in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)

   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)


# Failed non-cached task with --output-logs=errors-only should show output
  $ ${TURBO} run nocachebuilderror --output-logs=errors-only
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running nocachebuilderror in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  app-a:nocachebuilderror: cache bypass, force executing [0-9a-f]+ (re)
  app-a:nocachebuilderror:
  app-a:nocachebuilderror: > nocachebuilderror
  app-a:nocachebuilderror: > echo nocachebuilderror-app-a && exit 1
  app-a:nocachebuilderror:
  app-a:nocachebuilderror: nocachebuilderror-app-a
  app-a:nocachebuilderror: npm ERR! Lifecycle script `nocachebuilderror` failed with error:
  app-a:nocachebuilderror: npm ERR! Error: command failed
  app-a:nocachebuilderror: npm ERR!   in workspace: app-a
  app-a:nocachebuilderror: npm ERR!   at location: .* (re)
  app-a:nocachebuilderror: ERROR: command finished with error: command .*npm(?:\.cmd)? run nocachebuilderror exited \(1\) (re)
  app-a#nocachebuilderror: command .*npm(?:\.cmd)? run nocachebuilderror exited \(1\) (re)

   Tasks:    0 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  Failed:    app-a#nocachebuilderror

   ERROR  run failed: command  exited (1)
  [1]
