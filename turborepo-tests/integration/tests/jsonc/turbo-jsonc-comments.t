Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Remove turbo.json to avoid conflict with turbo.jsonc
  $ rm -f turbo.json

# Test that turbo.jsonc with comments is properly parsed
Create turbo.jsonc with comments
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

# Run turbo test to verify that the test task from the config is properly parsed
  $ ${TURBO} test --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running test in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Run turbo build to verify that the build task from the config is properly parsed
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Test that complex comments with special characters are handled correctly
Create turbo.jsonc with complex comments
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/complex-comments.jsonc turbo.jsonc

# Run turbo build to verify the config with complex comments is properly parsed
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running build in 3 packages (esc)
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Cached:    0 cached, 2 total
  Time: *s (re) 
