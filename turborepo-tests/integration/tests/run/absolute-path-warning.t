Setup
  $ . ${TESTDIR}/../../../helpers/setup.sh
  $ . ${TESTDIR}/../_helpers/setup_monorepo.sh $(pwd)

Expect warnings
  $ cp ${TESTDIR}/../_fixtures/turbo-configs/abs-path-globs.json $PWD/turbo.json
  $ git commit --quiet -am "Add turbo.json with absolute path in outputs"

  $ ${TURBO} build -v --dry 1> /dev/null 2> tmp.logs
Only check contents that comes after the warning prefix
We omit duplicates as Go with debug assertions enabled parses turbo.json twice
  $ grep -o "\[WARNING\].*" tmp.logs | sort -u
  [WARNING] Using an absolute path in "globalDependencies" (/an/absolute/path) will not work and will be an error in a future version
  [WARNING] Using an absolute path in "inputs" (/some/absolute/path) will not work and will be an error in a future version
  [WARNING] Using an absolute path in "outputs" (/another/absolute/path) will not work and will be an error in a future version
