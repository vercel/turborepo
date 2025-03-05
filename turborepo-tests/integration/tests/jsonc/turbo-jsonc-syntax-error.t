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
  
    x Failed to parse turbo.jsonc.
    |->   x Expected a property but instead found ','.
    |      ,-[turbo.jsonc:2:48]
    |    1 | {
    |    2 |   "$schema": "https://turbo.build/schema.json",,
    |      :                                                ^
    |    3 |   "globalDependencies": ["foo.txt"],
    |      `----
    |->   x expected `,` but instead found `42`
    |       ,-[turbo.jsonc:11:46]
    |    10 |     // this comment verifies that turbo can read .json files with comments
    |    11 |     "my-app#build": {
    |       :                                                  ^^
    |    12 |       "outputs": ["banana.txt", "apple.json"]42,
    |       `----
    `->   x expected `,` but instead found `}`
            ,-[turbo.jsonc:13:5]
         12 |       "outputs": ["banana.txt", "apple.json"]42,
         13 |       "inputs": [".env.local"
            :     ^
         14 |     },
            `---- 
