Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Remove turbo.json to avoid conflict with turbo.jsonc
  $ rm -f turbo.json

# Test that package-specific tasks in turbo.jsonc files work correctly
# First, create a repo structure with a package
  $ mkdir -p apps/my-app

# Create a root turbo.jsonc file with basic configuration
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

# Create a package-specific turbo.jsonc file
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/package-task.jsonc apps/my-app/turbo.jsonc

# Run turbo build, it should use both the root and package configs
  $ ${TURBO} build --output-logs=none
  • Packages in scope: my-app
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
