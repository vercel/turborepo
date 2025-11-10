Setup
  $ . ${TESTDIR}/../../helpers/setup_integration_test.sh
  $ jq '.engines = {"node": ">=12"}' package.json > package.json.new
  $ mv package.json.new package.json

Check a hash
  $ ${TURBO} build --dry=json --filter=my-app | jq '.tasks | last | .hash'
  "56d7eb9a31d82ee0"
Change engines
  $ jq '.engines = {"node": ">=16"}' package.json > package.json.new
  $ mv package.json.new package.json

Verify hash has changed
  $ ${TURBO} build --dry=json --filter=my-app | jq ".tasks | last | .hash"
  "1d8b8596ae37a40c"

Verify engines are part of global cache inputs
  $ ${TURBO} build --dry=json | jq '.globalCacheInputs.engines'
  {
    "node": ">=16"
  }
