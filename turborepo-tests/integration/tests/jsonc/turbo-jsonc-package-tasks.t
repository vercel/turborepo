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

# Run turbo build, it should show an error about unnecessary package task syntax
  $ ${TURBO} build --output-logs=none 2> error.txt
  [1]
  $ cat error.txt
    x Invalid turbo.json configuration
    |-> unnecessary_package_task_syntax (https://turbo.build/messages/unnecessary-package-task-syntax)
    |   
    |     x "my-app#build". Use "build" instead.
    |       ,-
    |     5 |         // Package-specific task with comments
    |     6 | ,->     "my-app#build": {
    |     7 | |         "outputs": [
    |     8 | |           "banana.txt", // Output file
    |     9 | |           "apple.json" // Another output file
    |    10 | |         ],
    |    11 | |         "inputs": [
    |    12 | |           "$TURBO_DEFAULT$", // Default inputs
    |    13 | |           ".env.local" // Environment file
    |    14 | |         ]
    |    15 | |->     }
    |       : `---- unnecessary package syntax found here
    |    16 |       }
    |       `----
    `->   x No "extends" key found.
            ,-
          1 | ,-> {
          2 | |     "$schema": "https://turbo.build/schema.json",
          3 | |     // This is a comment in turbo.jsonc
          4 | |     "tasks": {
          5 | |       // Package-specific task with comments
          6 | |       "my-app#build": {
          7 | |         "outputs": [
          8 | |           "banana.txt", // Output file
          9 | |           "apple.json" // Another output file
         10 | |         ],
         11 | |         "inputs": [
         12 | |           "$TURBO_DEFAULT$", // Default inputs
         13 | |           ".env.local" // Environment file
         14 | |         ]
         15 | |       }
         16 | |     }
         17 | |-> }
            : `---- add extends key here
            `----

# Now fix the package-specific turbo.jsonc file by adding extends
  $ cat > apps/my-app/turbo.jsonc << 'EOF'
{
  "$schema": "https://turbo.build/schema.json",
  "extends": ["//"],
  // This is a comment in turbo.jsonc
  "tasks": {
    // Package-specific task with comments
    "build": {
      "outputs": [
        "banana.txt", // Output file
        "apple.json" // Another output file
      ],
      "inputs": [
        "$TURBO_DEFAULT$", // Default inputs
        ".env.local" // Environment file
      ]
    }
  }
}
EOF

# Run turbo build again, it should now work
  $ ${TURBO} build --output-logs=none
  • Packages in scope: another, my-app, util (esc)
  • Running build in 3 packages (esc)
  • Remote caching disabled
  
  Tasks:    2 successful, 2 total
  Cached:    0 cached, 2 total
  Time:    *ms (re)
