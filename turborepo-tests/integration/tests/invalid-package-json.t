Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
Clear name field
  $ jq '.name = ""' apps/my-app/package.json > package.json.new
  $ mv package.json.new apps/my-app/package.json
Build should fail due to missing name field
  $ ${TURBO} build 1> ERR
  [1]
  $ grep -F --quiet 'x package.json must have a name field:' ERR
