Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Test that turbo falls back to turbo.jsonc when turbo.json doesn't exist
# First, create a repo with no turbo.json or turbo.jsonc
  $ rm -f turbo.json turbo.jsonc

# Try to run turbo build, it should fail because no config exists
  $ ${TURBO} build 2> error.txt
  [1]
  $ grep -q "Could not find turbo.json or turbo.jsonc" error.txt

# Now add a turbo.jsonc file
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

# Run turbo build, it should succeed using the turbo.jsonc file
  $ ${TURBO} build
  • Packages in scope: my-app
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Test that turbo prefers turbo.json over turbo.jsonc when both exist
# First, create a different turbo.json file
  $ cat > different-config.json << 'EOF'
{
  "$schema": "https://turbo.build/schema.json",
  "globalDependencies": ["different-dep.json"],
  "pipeline": {
    "special-task": {
      "outputs": ["special-output/**"]
    }
  }
}
EOF

  $ mv different-config.json turbo.json

# Run turbo special-task, it should use the config from turbo.json
  $ ${TURBO} special-task
  • Packages in scope: my-app
  • Running special-task in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Run turbo test, it should fail because test is only defined in turbo.jsonc
  $ ${TURBO} test 2> error.txt
  [1]
  $ grep -q "Could not find task" error.txt 
