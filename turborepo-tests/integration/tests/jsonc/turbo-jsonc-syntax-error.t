Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

# Remove turbo.json to avoid conflict with turbo.jsonc
  $ rm -f turbo.json

# Test that syntax errors in turbo.jsonc are properly reported
Create turbo.jsonc with syntax errors
  $ cp ${TESTDIR}/../../../integration/fixtures/turbo-configs/syntax-error.json turbo.jsonc

# Run turbo build to verify the syntax error is properly reported
  $ ${TURBO} build 2> error.txt
  [1]
  $ cat error.txt
  turbo_json_parse_error
  
    x Failed to parse turbo.json.
    |->   x Expected a property but instead found ','.
    |      ,-
    |    1 | {
    |    2 |   "$schema": "https://turbo.build/schema.json",,
    |      :                                                ^
    |    3 |   "globalDependencies": ["foo.txt"],
    |      `----
    |->   x expected `,` but instead found `42`
    |       ,-
    |    11 |     "my-app#build": {
    |    12 |       "outputs": ["banana.txt", "apple.json"]42,
    |       :                                              ^^
    |    13 |       "inputs": [".env.local"
    |       `----
    `->   x expected `,` but instead found `}`
            ,-
         13 |       "inputs": [".env.local"
         14 |     },
            :     ^
         15 |
            `----
  