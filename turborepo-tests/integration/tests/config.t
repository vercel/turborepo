Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Run test run
  $ ${TURBO} info --json | jq .config
  {
    "apiUrl": null,
    "loginUrl": null,
    "teamSlug": null,
    "teamId": null,
    "token": null,
    "signature": null,
    "preflight": null,
    "timeout": null,
    "enabled": null
  }

Run test run with api overloaded
  $ ${TURBO} info --json --api http://localhost:8000 | jq .config.apiUrl
  "http://localhost:8000"

Run test run with token overloaded
  $ ${TURBO} info --json --token 1234567890 | jq .config.token
  "1234567890"

Run test run with token overloaded from both TURBO_TOKEN and VERCEL_ARTIFACTS_TOKEN
  $ TURBO_TOKEN=turbo VERCEL_ARTIFACTS_TOKEN=vercel ${TURBO} info --json | jq .config.token
  "vercel"

Run test run with team overloaded
  $ ${TURBO} info --json --team vercel | jq .config.teamSlug
  "vercel"

Run test run with team overloaded from both env and flag (flag should take precedence)
  $ TURBO_TEAM=vercel ${TURBO} info --json --team turbo | jq .config.teamSlug
  "turbo"

Run test run with remote cache timeout env variable set
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} info --json | jq .config.timeout
  123

Run test run with remote cache timeout from both env and flag (flag should take precedence)
  $ TURBO_REMOTE_CACHE_TIMEOUT=123 ${TURBO} info --json --remote-cache-timeout 456 | jq .config.timeout
  456

Add turbo.json with unnecessary package task syntax to a package
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd)/apps/my-app "package-task.json"

Run build with package task in non-root turbo.json
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build
    x invalid turbo json
  
  Error: unnecessary_package_task_syntax (https://turbo.build/messages/unnecessary-package-task-syntax)
  
    x "my-app#build". Use "build" instead
      ,-[7:1]
    7 |         // this comment verifies that turbo can read .json files with comments
    8 | ,->     "my-app#build": {
    9 | |         "outputs": ["banana.txt", "apple.json"],
   10 | |         "dotEnv": [".env.local"]
   11 | |->     }
      : `---- unnecessary syntax found here
   12 |       }
      `----
  
  [1]

Remove unnecessary package task syntax
  $ rm $(pwd)/apps/my-app/turbo.json

Use our custom turbo config with an invalid env var
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd) "invalid-env-var.json"

Run build with invalid env var
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build
  invalid_env_prefix (https://turbo.build/messages/invalid-env-prefix)
  
    x Environment variables should not be prefixed with "$"
     ,-[6:1]
   6 |     "build": {
   7 |       "env": ["NODE_ENV", "$FOOBAR"],
     :                           ^^^^|^^^^
     :                               `-- variable with invalid prefix declared here
   8 |       "outputs": []
     `----
  
  [1]

Run in single package mode even though we have a task with package syntax
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build --single-package
  package_task_in_single_package_mode (https://turbo.build/messages/package-task-in-single-package-mode)
  
    x Package tasks (<package>#<task>) are not allowed in single-package
    | repositories: found //#something
      ,-[16:1]
   16 |     "something": {},
   17 |     "//#something": {},
      :                     ^|
      :                      `-- package task found here
   18 | 
      `----
  
  [1]

Use our custom turbo config with syntax errors
  $ . ${TESTDIR}/../../helpers/replace_turbo_config.sh $(pwd) "syntax-error.json"

Run build with syntax errors in turbo.json
  $ EXPERIMENTAL_RUST_CODEPATH=true ${TURBO} build
  turbo_json_parse_error
  
    x failed to parse turbo json
  
  Error: turbo_json_parse_error
  
    x Expected a property but instead found ','.
     ,-[1:1]
   1 | {
   2 |   "$schema": "https://turbo.build/schema.json",,
     :                                                ^
   3 |   "globalDependencies": ["foo.txt"],
     `----
  Error: turbo_json_parse_error
  
    x expected `,` but instead found `42`
      ,-[11:1]
   11 |     "my-app#build": {
   12 |       "outputs": ["banana.txt", "apple.json"]42,
      :                                              ^^
   13 |       "dotEnv": [".env.local"
      `----
  Error: turbo_json_parse_error
  
    x expected `,` but instead found `}`
      ,-[13:1]
   13 |       "dotEnv": [".env.local"
   14 |     },
      :     ^
   15 | 
      `----
  
  [1]
