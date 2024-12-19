Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Choose our custom config based on OS, since the input/output configs will be different  
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-inputs-win.json" || CONFIG="abs-path-inputs.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build > tmp.log 2>&1
  [1]
  $ grep --quiet '`inputs` cannot contain an absolute path' tmp.log

Choose our custom config based on OS, since the input/output configs will be different
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-outputs-win.json" || CONFIG="abs-path-outputs.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build > tmp.log 2>&1
  [1]
  $ grep --quiet '`outputs` cannot contain an absolute path' tmp.log

Choose our custom config based on OS, since the input/output configs will be different
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-global-deps-win.json" || CONFIG="abs-path-global-deps.json"

Copy config into the root of our monorepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Run build
  $ ${TURBO} build > tmp.log 2>&1
  [1]
  $ grep --quiet '`globalDependencies` cannot contain an absolute path' tmp.log
