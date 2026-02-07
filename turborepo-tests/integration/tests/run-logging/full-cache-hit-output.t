# Test that logs are printed on full cache hit
# This test verifies that when all tasks are cached (FULL TURBO), 
# the output logs are still printed correctly.
#
# Related to issue #9470 - TUI flicker fix should not break output on full cache hits

Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh --no-install

Run build once to populate the cache
  $ ${TURBO} run build --output-logs=none
  \xe2\x80\xa2 Packages in scope: another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
    Time:\s*[\.0-9]+m?s  (re)
  
   WARNING  no output files found for task my-app#build. Please check your `outputs` key in `turbo.json`


Run build again with --output-logs=full - should be FULL TURBO (all cached) and still show output
The output should contain cache hit messages and the echoed "building" from both tasks
  $ ${TURBO} run build --output-logs=full 2>&1 | grep -c "cache hit, replaying logs"
  2

Verify the actual build output is shown (both tasks echo "building")
  $ ${TURBO} run build --output-logs=full 2>&1 | grep -c "^.*:build: building$"
  2

Verify FULL TURBO is shown
  $ ${TURBO} run build --output-logs=full 2>&1 | grep -c "FULL TURBO"
  1

Verify --output-logs=hash-only shows status on full cache hit (suppresses logs)
  $ ${TURBO} run build --output-logs=hash-only 2>&1 | grep -c "cache hit, suppressing logs"
  2

Verify --output-logs=none shows minimal output on full cache hit  
  $ ${TURBO} run build --output-logs=none
  \xe2\x80\xa2 Packages in scope: another, my-app, util (esc)
  \xe2\x80\xa2 Running build in 3 packages (esc)
  \xe2\x80\xa2 Remote caching disabled (esc)
  
   Tasks:    2 successful, 2 total
  Cached:    2 cached, 2 total
    Time:\s*[\.0-9]+m?s >>> FULL TURBO (re)
  
