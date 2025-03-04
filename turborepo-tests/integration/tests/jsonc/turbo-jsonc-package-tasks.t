Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Test that package-specific tasks in turbo.jsonc files work correctly
# First, create a repo structure with a package
  $ mkdir -p apps/my-app

# Create a root turbo.jsonc file with basic configuration
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

# Create a package-specific turbo.jsonc file
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/package-task.jsonc apps/my-app/turbo.jsonc

# Run turbo build, it should use both the root and package configs
  $ ${TURBO} build
  • Packages in scope: my-app
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Create a package-specific task in the root turbo.jsonc
  $ cat > root-with-package-task.jsonc << 'EOF'
  {
    "$schema": "https://turbo.build/schema.json",
    // Root config with package-specific task
    "pipeline": {
      // Global build task
      "build": {
        "dependsOn": ["^build"],
        "outputs": ["dist/**"]
      },
      // Package-specific task
      "my-app#special": {
        "outputs": ["special/**"], // Special outputs
        "cache": false // Don't cache
      }
    }
  }
  EOF

  $ mv root-with-package-task.jsonc turbo.jsonc

# Run the package-specific task
  $ ${TURBO} run my-app#special
  • Packages in scope: my-app
  • Running special in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re) 
