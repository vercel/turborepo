Setup
  $ . ${TESTDIR}/../../../helpers/setup_integration_test.sh

Choose our custom config based on OS, since the input/output configs will be different  
  $ [[ "$OSTYPE" == "msys" ]] && CONFIG="abs-path-globs-win.json" || CONFIG="abs-path-globs.json"

Copy config into the root of our monrepo
  $ ${TESTDIR}/../../../helpers/replace_turbo_json.sh $PWD $CONFIG

Only check contents that comes after the warning prefix
We omit duplicates as Go with debug assertions enabled parses turbo.json twice
  $ ${TURBO} build -v --dry 1> /dev/null 2> tmp.logs

  $ grep -o "\[WARNING\].*" tmp.logs | sort -u
  \[WARNING] Using an absolute path in \"globalDependencies\" \(([A-Z]:\\an\\absolute\\path|/an/absolute/path)\) will not work and will be an error in a future version (re)
  \[WARNING] Using an absolute path in \"outputs\" \(([A-Z]:\\another\\absolute\\path|/another/absolute/path)\) will not work and will be an error in a future version (re)
