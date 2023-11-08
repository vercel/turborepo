Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Choose our custom config based on OS, since the input/output configs will be different
  $ if [[ "$OSTYPE" == "msys" ]]; then CONFIG="abs-path-globs-win.json"; else CONFIG="abs-path-globs.json"; fi

Copy config into the root of our monrepo
  $ cp "${TESTDIR}/bad-configs/${CONFIG}" $PWD/turbo.json
  $ cp ${TESTDIR}/../_fixtures/turbo-configs/$CONFIG $PWD/turbo.json
dos2unix the new file if we're on Windows
  $ if [[ "$OSTYPE" == "msys" ]]; then dos2unix --quiet "$PWD/turbo.json"; fi
  $ git commit --quiet -am "Add turbo.json with absolute path in outputs"

Expect warnings
  $ ${TURBO} build -v --dry > /dev/null
  [-0-9:.TWZ+]+ \[INFO]  turbo: skipping turbod since we appear to be in a non-interactive context (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "outputs" \(/another/absolute/path\) will not work and will be an error in a future version (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "inputs" \(/some/absolute/path\) will not work and will be an error in a future version (re)
  [0-9]{4}/[0-9]{2}/[0-9]{2} [-0-9:.TWZ+]+ \[WARNING] Using an absolute path in "globalDependencies" \(/an/absolute/path\) will not work and will be an error in a future version (re)
