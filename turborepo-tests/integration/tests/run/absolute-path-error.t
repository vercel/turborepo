Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Choose our custom config based on OS, since the input/output configs will be different  
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-inputs-win.json" || CONFIG="abs-path-inputs.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build
    x `inputs` cannot contain an absolute path
     ,-[5:1]
   5 |     "build": {
   6 |       "inputs": ["/another/absolute/path", "a/relative/path"]
     :                  ^^^^^^^^^^^^|^^^^^^^^^^^
     :                              `-- absolute path found here
   7 |     }
     `----
  
  [1]

Choose our custom config based on OS, since the input/output configs will be different
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-outputs-win.json" || CONFIG="abs-path-outputs.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build
    x `outputs` cannot contain an absolute path
     ,-[5:1]
   5 |     "build": {
   6 |       "outputs": ["/another/absolute/path", "a/relative/path"]
     :                   ^^^^^^^^^^^^|^^^^^^^^^^^
     :                               `-- absolute path found here
   7 |     }
     `----
  
  [1]


Choose our custom config based on OS, since the input/output configs will be different
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-global-deps-win.json" || CONFIG="abs-path-global-deps.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build
    x `globalDependencies` cannot contain an absolute path
     ,-[2:1]
   2 |   "$schema": "https://turbo.build/schema.json",
   3 |   "globalDependencies": ["/an/absolute/path", "some/file"],
     :                          ^^^^^^^^^|^^^^^^^^^
     :                                   `-- absolute path found here
   4 |   "pipeline": {
     `----
  
  [1]
