Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

# Test that syntax errors in turbo.jsonc are properly reported
Create turbo.jsonc with syntax errors
  $ cat > syntax-error.jsonc << EOF
  {
    "$schema": "https://turbo.build/schema.json",,
    // Comment with a syntax error below
    "globalDependencies": [
      "tsconfig.json"
    ],
    "pipeline": {
      "build": {
        "dependsOn": [
          "^build"
        ],
        "outputs": ["dist/**"]42,
        "inputs": [".env.local"
      },
    }
  }
  EOF

  $ mv syntax-error.jsonc turbo.jsonc

# Run turbo build to verify the syntax error is properly reported
  $ ${TURBO} build 2> error.txt
  [1]
  $ cat error.txt
  turbo_json_parse_error
  
    x Failed to parse turbo.jsonc.
    |->   x Expected a property but instead found ','.
    |      ,-[turbo.jsonc:2:48]
    |    1 | {
    |    2 |   "$schema": "https://turbo.build/schema.json",,
    |      :                                                ^
    |    3 |   // Comment with a syntax error below
    |      `----
    |->   x expected `,` but instead found `42`
    |       ,-[turbo.jsonc:12:46]
    |    11 |         ],
    |    12 |         "outputs": ["dist/**"]42,
    |       :                                  ^^
    |    13 |         "inputs": [".env.local"
    |       `----
    `->   x expected `,` but instead found `}`
            ,-[turbo.jsonc:15:5]
         14 |       },
         15 |     }
            :     ^
         16 |   }
            `---- 
