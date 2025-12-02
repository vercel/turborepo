Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh run_logging

# Run with --listTests --json passed to the task
# This simulates the VS Code Jest extension behavior.
# After fix, it should NOT show prefixes.

  $ ${TURBO} run build -- --listTests --json
  > build
  > echo build-app-a --listTests --json
  > build
  > echo build-app-a --listTests --json
  \xe2\x80\xa2 Packages in scope: app-a (esc)
  \xe2\x80\xa2 Running build in 1 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  cache miss, executing [a-f0-9]+ (re)
  
  > build
  > echo build-app-a --listTests --json
  
  build-app-a --listTests --json
  
   Tasks:    1 successful, 1 total
  Cached:    0 cached, 1 total
    Time:\s*[\.0-9]+m?s  (re)
  
  /bin/bash: line 5: build: command not found
  build-app-a --listTests --json
  /bin/bash: line 7: build: command not found
  build-app-a --listTests --json
