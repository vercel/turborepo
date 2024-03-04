Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh

Add turbo.json with unnecessary package task syntax to a package
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd)/apps/my-app "package-task.json"

Run build with package task in non-root turbo.json
  $ ${TURBO} build
    x invalid turbo json
  
  Error: unnecessary_package_task_syntax (https://turbo.build/messages/unnecessary-package-task-syntax)
  
    x "my-app#build". Use "build" instead
      ,-\[apps[\\/]my-app[\\/]turbo.json:7:1\] (re)
    7 |         // this comment verifies that turbo can read .json files with comments
    8 | ,->     "my-app#build": {
    9 | |         "outputs": ["banana.txt", "apple.json"],
   10 | |         "dotEnv": [".env.local"]
   11 | |->     }
      : `---- unnecessary package syntax found here
   12 |       }
      `----
  
  [1]




Remove unnecessary package task syntax
  $ rm $(pwd)/apps/my-app/turbo.json

Use our custom turbo config with an invalid env var
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd) "invalid-env-var.json"

Run build with invalid env var
  $ ${TURBO} build
  invalid_env_prefix (https://turbo.build/messages/invalid-env-prefix)
  
    x Environment variables should not be prefixed with "$"
     ,-[turbo.json:6:1]
   6 |     "build": {
   7 |       "env": ["NODE_ENV", "$FOOBAR"],
     :                           ^^^^|^^^^
     :                               `-- variable with invalid prefix declared here
   8 |       "outputs": []
     `----
  
  [1]



Run in single package mode even though we have a task with package syntax
  $ ${TURBO} build --single-package
  package_task_in_single_package_mode (https://turbo.build/messages/package-task-in-single-package-mode)
  
    x Package tasks (<package>#<task>) are not allowed in single-package
    | repositories: found //#something
      ,-[turbo.json:16:1]
   16 |     "something": {},
   17 |     "//#something": {},
      :                     ^|
      :                      `-- package task found here
   18 | 
      `----
  
  [1]



Use our custom turbo config with syntax errors
  $ . ${TESTDIR}/../../helpers/replace_turbo_json.sh $(pwd) "syntax-error.json"

Run build with syntax errors in turbo.json
  $ ${TURBO} build
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
