Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Test that turbo.jsonc with comments is properly parsed
Create turbo.jsonc with comments
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/basic.jsonc turbo.jsonc

# Run turbo test to verify that the test task from the config is properly parsed
  $ ${TURBO} test
  • Packages in scope: my-app
  • Running test in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Run turbo build to verify that the build task from the config is properly parsed
  $ ${TURBO} build
  • Packages in scope: my-app
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re)
  

# Test that complex comments with special characters are handled correctly
Create turbo.jsonc with complex comments
  $ cat > complex-comments.jsonc << 'EOF'
  {
    "$schema": "https://turbo.build/schema.json",
    /* Multi-line comment with special characters:
     * "quotes", 'single quotes', [brackets], {braces}
     */
    "globalDependencies": [
      "tsconfig.json" // Comment with "quotes"
    ],
    "pipeline": {
      "build": {
        // Comment with JSON-like content: { "key": "value" }
        "dependsOn": [
          "^build" /* Comment with /* nested comment syntax */ 
        ],
        "outputs": [
          ".next/**", // Comment with // in it
          "dist/**"   /* Comment with /* in it */
        ]
      }
    }
  }
  EOF

  $ mv complex-comments.jsonc turbo.jsonc

# Run turbo build to verify the config with complex comments is properly parsed
  $ ${TURBO} build
  • Packages in scope: my-app
  • Running build in 1 packages
  • Remote caching disabled
  
  Tasks:  1 successful, 0 total
  Time: *s (re) 
