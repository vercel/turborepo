Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Use our custom turbo config with syntax errors
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd) "syntax-error.json"

Run build with invalid turbo.json
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build
   ERROR  run failed: failed to parse turbo json
  turbo_json::parser::parse_error
  
    x failed to parse turbo json
  
  Error: turbo_json::parser::parse_error
  
    x Expected a property but instead found ','.
     ,-[1:1]
   1 | {
   2 |   "$schema": "https://turbo.build/schema.json",,
     :                                                ^
   3 |   "globalDependencies": ["foo.txt"],
     `----
  Error: turbo_json::parser::parse_error
  
    x expected `,` but instead found `42`
      ,-[11:1]
   11 |     "my-app#build": {
   12 |       "outputs": ["banana.txt", "apple.json"]42,
      :                                              ^^
   13 |       "dotEnv": [".env.local"
      `----
  Error: turbo_json::parser::parse_error
  
    x expected `,` but instead found `}`
      ,-[13:1]
   13 |       "dotEnv": [".env.local"
   14 |     },
      :     ^
   15 | 
      `----
  
  [1]



